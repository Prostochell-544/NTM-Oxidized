//! NTM through Horizon's public mod API only: the trait hooks and `Host`, no mixins. The logic is
//! in `ntm_core`. Where the API has no way yet, a stub stands in, marked `API gap`.

use horizon_client::mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, BlockEntityContainerEvent, BlockEntityEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, ChunkLoadEvent, ChunkUnloadEvent, Compound,
    Flow, Holder, Hooks, Host, ItemStack, LevelTickPreEvent, MenuSlotRulesEvent, ModBlockEntity,
    ModContainer, NativeScreen, ScreenDrawEvent, ScreenEvent, ScreenTooltipEvent,
    ServerDataLoadedEvent, ServerTickEvent, Tag, Text, UseItemOnEvent, WorldLifecycleEvent,
    WorldPhase,
};
use ntm_core::energy::{EnergyHandlers, NodeId, Nodespace, Pos, Port, around};
use ntm_core::machine::{FuelKind, RecipeCache, Shredder, ShredderRules, WoodBurner};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, Once};

const CABLE: &str = "ntm:red_cable";
const WOOD_BURNER: &str = "ntm:machine_wood_burner";
const SHREDDER: &str = "ntm:machine_shredder";
const NET_TOOL: &str = "ntm:power_net_tool";
const BLADES_PREFIX: &str = "ntm:blades_";
const SCRAP: &str = "ntm:scrap";
const SHREDDER_RECIPE: &str = "ntm:shredder";
const SHREDDER_MENU: &str = "ntm:shredder";
const SHREDDER_GUI: &str = "ntm:gui/gui_shredder";
const FUEL_SLOT: usize = 1;
// The game's log keeps info lines only from targets starting with "horizon".
const LOG: &str = "horizon_ntm_api";

/// A machine's power. The API can't reach another block entity's own data (`with_mod_block_entity`
/// isn't in `Host`), but it can reach its attachments, so the network reads and writes this.
static POWER: AttachmentType<i64> = AttachmentType {
    namespace: "ntm",
    name: "power",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |power| Ok([("power".into(), Tag::Long(*power))].into()),
        decode: |data: &Compound| match data.get(&"power".into()) {
            Some(Tag::Long(power)) => Ok(*power),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

#[derive(Default)]
struct Level {
    space: Nodespace<Pos>,
    cables: HashMap<Pos, NodeId>,
    ports: HashMap<Pos, Port>,
    unloaded: HashSet<[i32; 2]>,
    random: u64,
}

impl Level {
    /// Removes what the block at each position no longer is. A chunk that left the world keeps
    /// its cables' nodes, so a network still runs through it, as in NTM.
    fn drop_broken(&mut self, host: &dyn Host) {
        let Level {
            space,
            cables,
            ports,
            unloaded,
            ..
        } = self;
        cables.retain(|&pos, node| {
            let keep = unloaded.contains(&chunk_of(pos)) || host.block(pos) == CABLE;
            if !keep {
                space.destroy_node(*node);
            }
            keep
        });
        ports.retain(|&pos, port| {
            let keep = is_machine(&host.block(pos));
            if !keep {
                port.disable_if_present(space);
            }
            keep
        });
    }
}

#[derive(Default)]
struct NtmHooks {
    levels: Mutex<HashMap<String, Level>>,
    shredder_results: Mutex<RecipeCache<ItemStack>>,
}

impl NtmHooks {
    fn with_level<R>(&self, dimension: &str, f: impl FnOnce(&mut Level) -> R) -> R {
        let mut levels = self.levels.lock().unwrap_or_else(|e| e.into_inner());
        f(levels.entry(dimension.to_owned()).or_default())
    }

    /// The recipes and tags were (re)built: what was found in them is stale.
    fn forget_recipes(&self) {
        self.shredder_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

fn chunk_of(pos: Pos) -> [i32; 2] {
    [pos[0] >> 4, pos[2] >> 4]
}

fn is_machine(block: &str) -> bool {
    block == WOOD_BURNER || block == SHREDDER
}

fn int(nbt: &Compound, key: &str) -> i32 {
    match nbt.get(&key.into()) {
        Some(Tag::Int(n)) => *n,
        _ => 0,
    }
}

fn max_power_of(block: &str) -> i64 {
    match block {
        WOOD_BURNER => WoodBurner::MAX_POWER,
        SHREDDER => Shredder::MAX_POWER,
        _ => 0,
    }
}

fn power(host: &mut dyn Host, pos: Pos) -> i64 {
    POWER.get(host, Holder::BlockEntity(pos)).unwrap_or(0)
}

fn set_power(host: &mut dyn Host, pos: Pos, power: i64) {
    let _ = POWER.set(host, Holder::BlockEntity(pos), power);
}

/// The machines of the level ticking, as the power network reads and changes them.
struct Handlers<'a>(RefCell<&'a mut dyn Host>);

impl EnergyHandlers<Pos> for Handlers<'_> {
    fn power(&self, pos: Pos) -> i64 {
        power(*self.0.borrow_mut(), pos)
    }

    fn set_power(&mut self, pos: Pos, value: i64) {
        set_power(*self.0.get_mut(), pos, value);
    }

    fn max_power(&self, pos: Pos) -> i64 {
        max_power_of(&self.0.borrow().block(pos))
    }

    fn is_bad_link(&self, pos: Pos) -> bool {
        !is_machine(&self.0.borrow().block(pos))
    }
}

/// API gap: no fuel burn time in `Host` (the game's `cooking_fuel` providers). Vanilla's main fuels.
fn base_burn_time(item: &str) -> i32 {
    let name = item.strip_prefix("minecraft:").unwrap_or("");
    match name {
        "lava_bucket" => 20_000,
        "coal_block" => 16_000,
        "blaze_rod" => 2_400,
        "coal" | "charcoal" => 1_600,
        "stick" => 100,
        _ if is_log(name) || name.ends_with("_planks") => 300,
        _ if name.ends_with("_slab") && is_wooden(name) => 150,
        _ if name.ends_with("_stairs") && is_wooden(name) => 300,
        _ => 0,
    }
}

/// API gap: no item tags in `Host` (`server_item_in_tag` uses gameplay-private types). By name.
fn is_log(name: &str) -> bool {
    ["_log", "_wood", "_stem", "_hyphae"]
        .iter()
        .any(|end| name.ends_with(end))
        && !name.contains("petrified")
}

fn is_wooden(name: &str) -> bool {
    [
        "oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "pale_oak",
        "bamboo", "crimson", "warped",
    ]
    .iter()
    .any(|wood| name.starts_with(wood))
}

fn fuel_burn_time(stack: &ItemStack) -> i32 {
    let name = stack.item.strip_prefix("minecraft:").unwrap_or("");
    let kind = if is_log(name) {
        FuelKind::Log
    } else if name.ends_with("_planks")
        || ((name.ends_with("_slab") || name.ends_with("_stairs")) && is_wooden(name))
        || name == "stick"
    {
        FuelKind::Wood
    } else {
        FuelKind::Other
    };
    WoodBurner::burn_time_of(base_burn_time(&stack.item), kind)
}

fn tick_wood_burner(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let mut burner = WoodBurner {
        power: power(host, pos),
        burn_time: int(&entity.nbt, "burnTime"),
        max_burn_time: int(&entity.nbt, "maxBurnTime"),
        // On until the machine has a screen with NTM's switch.
        is_on: true,
    };
    let fuel = entity.items[FUEL_SLOT].as_ref().map_or(0, fuel_burn_time);
    if burner.tick(fuel)
        && let Some(stack) = entity.items[FUEL_SLOT].as_mut()
    {
        stack.count -= 1;
        if stack.count == 0 {
            // API gap: no item data in `Host`, so no `crafting_remainder`. Vanilla's one fuel with one.
            let lava = &*stack.item == "minecraft:lava_bucket";
            entity.items[FUEL_SLOT] = lava.then(|| ItemStack {
                item: "minecraft:bucket".into(),
                count: 1,
                damage: 0,
                extra: None,
            });
        }
    }
    set_power(host, pos, burner.power);
    entity
        .nbt
        .insert("burnTime".into(), Tag::Int(burner.burn_time));
    entity
        .nbt
        .insert("maxBurnTime".into(), Tag::Int(burner.max_burn_time));
    // The furnace menu's flame: time left, total, then its arrow, unused.
    entity.data = vec![burner.burn_time, burner.max_burn_time, 0, 0];
}

struct Rules<'a>(&'a dyn Host, &'a Mutex<RecipeCache<ItemStack>>);

impl ShredderRules<ItemStack> for Rules<'_> {
    fn same_item(&self, a: &ItemStack, b: &ItemStack) -> bool {
        a.item == b.item && a.damage == b.damage && a.extra == b.extra
    }

    fn count(&self, stack: &ItemStack) -> u32 {
        stack.count
    }

    fn set_count(&self, stack: &mut ItemStack, count: u32) {
        stack.count = count;
    }

    fn max_stack(&self, stack: &ItemStack) -> u32 {
        self.0.max_stack_size(&stack.item).unwrap_or(64)
    }

    fn blade_damage(&self, stack: &ItemStack) -> Option<(u32, u32)> {
        // API gap: no item max damage in `Host`. The numbers ntm-content gives the blades.
        let max = match stack.item.strip_prefix(BLADES_PREFIX)? {
            "steel" => 200,
            "titanium" => 350,
            _ => 0,
        };
        Some((stack.damage, max))
    }

    fn set_damage(&self, stack: &mut ItemStack, damage: u32) {
        stack.damage = damage;
    }

    fn result(&self, stack: &ItemStack) -> Option<ItemStack> {
        let host = self.0;
        let mut results = self.1.lock().unwrap_or_else(|e| e.into_inner());
        results.get(&stack.item, || {
            host.recipes_of_type(SHREDDER_RECIPE)
                .iter()
                .find(|recipe| {
                    host.recipe_ingredient(&recipe.json["ingredient"])
                        .is_some_and(|ingredient| ingredient.test(stack))
                })
                .and_then(|recipe| host.recipe_result(&recipe.json["result"]))
                .or_else(|| host.recipe_result(&serde_json::json!({ "id": SCRAP })))
        })
    }
}

fn tick_shredder(
    host: &mut dyn Host,
    pos: Pos,
    entity: &mut ModBlockEntity,
    results: &Mutex<RecipeCache<ItemStack>>,
) {
    let mut shredder = Shredder {
        power: power(host, pos),
        progress: int(&entity.nbt, "progress"),
    };
    let rules = Rules(&*host, results);
    shredder.tick(&rules, &mut entity.items);
    let left = Shredder::gear(&rules, entity.items[Shredder::LEFT_BLADE].as_ref());
    let right = Shredder::gear(&rules, entity.items[Shredder::RIGHT_BLADE].as_ref());
    set_power(host, pos, shredder.power);
    entity
        .nbt
        .insert("progress".into(), Tag::Int(shredder.progress));
    entity.data = vec![shredder.power as i32, shredder.progress, left, right];
}

impl Hooks for NtmHooks {
    fn server_tick(&self, _host: &mut dyn Host, _event: &mut ServerTickEvent<'_>) {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            log::info!(
                target: LOG,
                "[ntm_api] {} ticking on thread {:?}",
                ntm_core::NAME,
                std::thread::current().name()
            );
        });
    }

    /// NTM updates its networks before the block entities tick.
    fn level_tick_pre(&self, host: &mut dyn Host, event: &mut LevelTickPreEvent<'_>) {
        self.with_level(event.dimension, |level| {
            level.drop_broken(&*host);
            let mut random = level.random;
            level
                .space
                .update(&mut Handlers(RefCell::new(&mut *host)), &mut |bound| {
                    random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                    ((random >> 33) as usize) % bound
                });
            level.random = random;
        });
    }

    fn world_lifecycle(&self, _host: &mut dyn Host, event: &mut WorldLifecycleEvent<'_>) {
        if matches!(
            event.phase,
            WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed
        ) {
            self.levels
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            self.forget_recipes();
        }
    }

    fn server_data_loaded(&self, _host: &mut dyn Host, _event: &mut ServerDataLoadedEvent) {
        self.forget_recipes();
    }

    fn chunk_load(&self, _host: &mut dyn Host, event: &mut ChunkLoadEvent<'_>) {
        self.with_level(event.dimension, |level| {
            level.unloaded.remove(&event.chunk);
        });
    }

    /// NTM takes a machine's ports down when its chunk unloads, and they come back when it ticks.
    fn chunk_unload(&self, _host: &mut dyn Host, event: &mut ChunkUnloadEvent<'_>) {
        self.with_level(event.dimension, |level| {
            level.unloaded.insert(event.chunk);
            let Level { space, ports, .. } = level;
            ports.retain(|&pos, port| {
                let keep = chunk_of(pos) != event.chunk;
                if !keep {
                    port.disable_if_present(space);
                }
                keep
            });
        });
    }

    fn block_entity(
        &self,
        _host: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == CABLE || is_machine(event.block))
            .then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == CABLE || is_machine(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _host: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        match event.block {
            // A furnace's menu until the machine has a screen of its own: only its fuel slot works.
            WOOD_BURNER => Some(ModContainer {
                slots: 3,
                faces: std::array::from_fn(|_| vec![FUEL_SLOT]),
                inputs: vec![FUEL_SLOT],
                outputs: vec![],
                menu: Some("minecraft:furnace".into()),
            }),
            SHREDDER => {
                let inputs: Vec<usize> = Shredder::INPUTS.collect();
                let outputs: Vec<usize> = Shredder::OUTPUTS.collect();
                Some(ModContainer {
                    slots: Shredder::SLOTS,
                    faces: std::array::from_fn(|_| {
                        inputs.iter().chain(&outputs).copied().collect()
                    }),
                    inputs,
                    outputs,
                    menu: Some(SHREDDER_MENU.into()),
                })
            }
            _ => None,
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        let pos = event.pos;
        let id = event.entity.id.clone();
        if id == CABLE {
            self.with_level(host.dimension(), |level| {
                let Level { space, cables, .. } = level;
                cables.entry(pos).or_insert_with(|| {
                    space
                        .node_at(pos)
                        .unwrap_or_else(|| space.create_node(vec![pos], around(pos)))
                });
            });
        } else if is_machine(&id) {
            self.with_level(host.dimension(), |level| {
                let Level { space, ports, .. } = level;
                let port = ports
                    .entry(pos)
                    .or_insert_with(|| Port::new(vec![pos], around(pos)));
                port.update(space, pos);
                if id == WOOD_BURNER {
                    port.check_provide(space, pos);
                } else {
                    port.check_subscribe(space, pos);
                }
            });
            if id == WOOD_BURNER {
                tick_wood_burner(host, pos, event.entity);
            } else {
                tick_shredder(host, pos, event.entity, &self.shredder_results);
            }
        }
    }

    /// `ItemPowerNetTool`: right click on a cable or machine for its network, plus what this
    /// machine holds, which the original shows on the machine's screen.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != NET_TOOL {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Cancel;
        };
        let pos = event.pos;
        let mut lines = self.with_level(host.dimension(), |level| {
            let net = level
                .space
                .node_at(pos)
                .and_then(|node| level.space.net_of(node));
            match net.and_then(|net| {
                Some((level.space.net_sizes(net)?, level.space.energy_tracker(net)?))
            }) {
                Some(((links, providers, receivers), flow)) => vec![
                    format!("Links: {links}"),
                    format!("Providers: {providers}"),
                    format!("Receivers: {receivers}"),
                    format!("Last flow: {flow} HE/t"),
                ],
                None => vec!["Error: No network found!".to_owned()],
            }
        });
        let max = max_power_of(&host.block(pos));
        if max > 0 {
            lines.push(format!("Power: {} / {max} HE", power(host, pos)));
        }
        for line in lines {
            host.tell(player.uuid, Text::literal(line));
        }
        Flow::Cancel
    }

    fn menu_slot_rules(&self, event: &mut MenuSlotRulesEvent<'_>) {
        if event.menu == SHREDDER_MENU {
            let blade = event.stack.item.starts_with(BLADES_PREFIX);
            event.may_place = match event.slot {
                0..=8 => !blade,
                27 | 28 => blade,
                _ => false,
            };
        }
    }

    fn screen(&self, _host: &dyn Host, event: &mut ScreenEvent<'_>) -> Option<NativeScreen> {
        if event.block != SHREDDER {
            return None;
        }
        let grid = |x: f32, slot: usize| {
            Some([
                x + (slot % 3) as f32 * 18.0,
                18.0 + (slot / 3) as f32 * 18.0,
            ])
        };
        let mut slots: Vec<Option<[f32; 2]>> = (0..9).map(|slot| grid(44.0, slot)).collect();
        slots.extend((0..18).map(|slot| grid(116.0, slot)));
        slots.extend([Some([44.0, 108.0]), Some([80.0, 108.0]), Some([8.0, 108.0])]);
        Some(NativeScreen {
            width: 176.0,
            height: 233.0,
            slots,
            inventory_x: 8.0,
            inventory_y: 151.0,
            own_labels: true,
            own_background: true,
        })
    }

    /// `GUIMachineShredder.drawGuiContainerBackgroundLayer` and `drawGuiContainerForegroundLayer`.
    fn screen_draw(&self, host: &dyn Host, event: &mut ScreenDrawEvent<'_>) {
        if event.block != SHREDDER {
            return;
        }
        let [power, progress, left, right] = *event.data else {
            return;
        };
        let origin = event.origin;
        let batch = &mut *event.batch;
        let mut part = |src: [f32; 2], dst: [f32; 2], size: [f32; 2]| {
            host.sprite(
                batch,
                SHREDDER_GUI,
                [src[0], src[1], size[0], size[1]],
                [origin[0] + dst[0], origin[1] + dst[1], size[0], size[1]],
            );
        };
        part([0.0, 0.0], [0.0, 0.0], [176.0, 233.0]);
        if power > 0 {
            let height = (i64::from(power) * 88 / Shredder::MAX_POWER) as f32;
            part([176.0, 160.0 - height], [8.0, 106.0 - height], [16.0, height]);
        }
        let width = (progress * 34 / Shredder::PROCESSING_SPEED) as f32;
        part([176.0, 54.0], [63.0, 89.0], [width + 1.0, 18.0]);
        for (gear, u, x) in [(left, 176.0, 43.0), (right, 194.0, 79.0)] {
            if gear != 0 {
                part([u, (gear - 1) as f32 * 18.0], [x, 71.0], [18.0, 18.0]);
            }
        }
        // The title is centred over the panel right of the power bar, not over the whole screen.
        const LABEL: u32 = 0x404040;
        if let Some(title) = host.screen_title() {
            // API gap: no text width in `Host`. Most of the font's glyphs are 6 pixels wide.
            let x = 106.0 - title.chars().count() as f32 * 3.0;
            host.text(batch, &title, [origin[0] + x, origin[1] + 6.0], LABEL, false);
        }
        // API gap: no translation in `Host`.
        host.text(
            batch,
            "Inventory",
            [origin[0] + 8.0, origin[1] + 139.0],
            LABEL,
            false,
        );
    }

    fn screen_tooltip(&self, _host: &dyn Host, event: &mut ScreenTooltipEvent<'_>) {
        if event.block != SHREDDER {
            return;
        }
        let [power, _, left, right] = *event.data else {
            return;
        };
        let at = event.at;
        if (8.0..24.0).contains(&at[0]) && (18.0..106.0).contains(&at[1]) {
            event
                .lines
                .push(format!("{power}/{} HE", Shredder::MAX_POWER));
        } else if (43.0..97.0).contains(&at[0])
            && (71.0..89.0).contains(&at[1])
            && [left, right].iter().any(|gear| matches!(gear, 0 | 3))
        {
            event
                .lines
                .push("Error: Shredder blades are broken or missing!".to_owned());
        }
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(NtmHooks::default())
}
