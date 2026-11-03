// The game's side of NTM's power network and machines, added as `game::ntm_direct`. The helpers
// beside this file call it from the hook points; the logic is in `ntm_core`.

use super::Game;
use crate::items::ItemStack;
use crate::text::Text;
use horizon_client_render::chunk::block_entity::{ModBlockEntity, ModContainer};
use horizon_world::nbt::{Compound, Tag};
use ntm_core::energy::{EnergyHandlers, NodeId, Nodespace, Pos, Port, around};
use crate::mod_api::{HudBatch, NativeScreen};
use ntm_core::machine::{FuelKind, RecipeCache, Shredder, ShredderRules, WoodBurner};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

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
    fn drop_broken(&mut self, game: &Game<'_>) {
        let Level {
            space,
            cables,
            ports,
            unloaded,
            ..
        } = self;
        cables.retain(|&pos, node| {
            let keep = unloaded.contains(&chunk_of(pos)) || game.native_block(pos) == CABLE;
            if !keep {
                space.destroy_node(*node);
            }
            keep
        });
        ports.retain(|&pos, port| {
            let keep = is_machine(&game.native_block(pos));
            if !keep {
                port.disable_if_present(space);
            }
            keep
        });
    }
}

static LEVELS: Mutex<Option<HashMap<String, Level>>> = Mutex::new(None);
static SHREDDER_RESULTS: Mutex<Option<RecipeCache<ItemStack>>> = Mutex::new(None);

fn with_level<R>(dimension: &str, f: impl FnOnce(&mut Level) -> R) -> R {
    let mut levels = LEVELS.lock().unwrap_or_else(|e| e.into_inner());
    let levels = levels.get_or_insert_with(HashMap::new);
    f(levels.entry(dimension.to_owned()).or_default())
}

fn chunk_of(pos: Pos) -> [i32; 2] {
    [pos[0] >> 4, pos[2] >> 4]
}

fn is_machine(block: &str) -> bool {
    block == WOOD_BURNER || block == SHREDDER
}

fn long(nbt: &Compound, key: &str) -> i64 {
    match nbt.get(&key.into()) {
        Some(Tag::Long(n)) => *n,
        _ => 0,
    }
}

fn int(nbt: &Compound, key: &str) -> i32 {
    match nbt.get(&key.into()) {
        Some(Tag::Int(n)) => *n,
        _ => 0,
    }
}

fn max_power_of(id: &str) -> i64 {
    match id {
        WOOD_BURNER => WoodBurner::MAX_POWER,
        SHREDDER => Shredder::MAX_POWER,
        _ => 0,
    }
}

/// The machines of the level ticking, as the power network reads and changes them.
struct Handlers<'a, 'g>(&'a mut Game<'g>);

impl EnergyHandlers<Pos> for Handlers<'_, '_> {
    fn power(&self, pos: Pos) -> i64 {
        self.0
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
            .and_then(|entity| match entity {
                crate::game::BlockEntity::Mod(m) => Some(long(&m.nbt, "power")),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn set_power(&mut self, pos: Pos, power: i64) {
        self.0.native_with_mod_block_entity(pos, |entity| {
            entity.nbt.insert("power".into(), Tag::Long(power));
        });
    }

    fn max_power(&self, pos: Pos) -> i64 {
        max_power_of(&self.0.native_block(pos))
    }

    fn is_bad_link(&self, pos: Pos) -> bool {
        !is_machine(&self.0.native_block(pos))
    }
}

/// `native_level_tick_pre`: NTM updates its networks before the block entities tick.
pub(super) fn level_tick(game: &mut Game<'_>, dimension: &str) {
    with_level(dimension, |level| {
        level.drop_broken(game);
        let mut random = level.random;
        level.space.update(&mut Handlers(game), &mut |bound| {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((random >> 33) as usize) % bound
        });
        level.random = random;
    });
}

pub(super) fn world_closed() {
    *LEVELS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    data_loaded();
}

/// The recipes and tags were (re)built: what was found in them is stale.
pub(super) fn data_loaded() {
    *SHREDDER_RESULTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(super) fn chunk_loaded(dimension: &str, chunk: [i32; 2]) {
    with_level(dimension, |level| {
        level.unloaded.remove(&chunk);
    });
}

/// NTM takes a machine's ports down when its chunk unloads, and they come back when it ticks.
pub(super) fn chunk_unloaded(dimension: &str, chunk: [i32; 2]) {
    with_level(dimension, |level| {
        level.unloaded.insert(chunk);
        let Level { space, ports, .. } = level;
        ports.retain(|&pos, port| {
            let keep = chunk_of(pos) != chunk;
            if !keep {
                port.disable_if_present(space);
            }
            keep
        });
    });
}

pub(super) fn block_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if entity.is_none() && (block == CABLE || is_machine(block)) {
        *entity = Some((block.to_owned(), Compound::new()));
    }
}

pub(super) fn block_entity_ticks(block: &str, ticks: &mut bool) {
    if block == CABLE || is_machine(block) {
        *ticks = true;
    }
}

pub(super) fn block_entity_container(block: &str, container: &mut Option<ModContainer>) {
    if block == WOOD_BURNER && container.is_none() {
        // A furnace's menu until the machine has a screen of its own: only its fuel slot works.
        *container = Some(ModContainer {
            slots: 3,
            faces: std::array::from_fn(|_| vec![FUEL_SLOT]),
            inputs: vec![FUEL_SLOT],
            outputs: vec![],
            menu: Some("minecraft:furnace".into()),
        });
    } else if block == SHREDDER && container.is_none() {
        let inputs: Vec<usize> = Shredder::INPUTS.collect();
        let outputs: Vec<usize> = Shredder::OUTPUTS.collect();
        *container = Some(ModContainer {
            slots: Shredder::SLOTS,
            faces: std::array::from_fn(|_| inputs.iter().chain(&outputs).copied().collect()),
            inputs,
            outputs,
            menu: Some(SHREDDER_MENU.into()),
        });
    }
}

pub(super) fn tick(game: &mut Game<'_>, pos: Pos, entity: &mut ModBlockEntity) {
    let id = entity.id.clone();
    if id == CABLE {
        with_level(game.native_dimension(), |level| {
            let Level { space, cables, .. } = level;
            cables.entry(pos).or_insert_with(|| {
                space
                    .node_at(pos)
                    .unwrap_or_else(|| space.create_node(vec![pos], around(pos)))
            });
        });
    } else if is_machine(&id) {
        with_level(game.native_dimension(), |level| {
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
            tick_wood_burner(game, entity);
        } else {
            tick_shredder(game, entity);
        }
    }
}

fn fuel_burn_time(game: &Game<'_>, stack: &ItemStack) -> i32 {
    let base = game.res.providers().burn_time(game.items(), stack, false);
    let in_tag = |tag| game.native_server_item_in_tag(stack, tag).unwrap_or(false);
    // The ore dictionary names NTM reads: log* and *Wood*.
    let kind = if in_tag("minecraft:logs") {
        FuelKind::Log
    } else if in_tag("minecraft:planks")
        || in_tag("minecraft:wooden_slabs")
        || in_tag("minecraft:wooden_stairs")
        || &*stack.item == "minecraft:stick"
    {
        FuelKind::Wood
    } else {
        FuelKind::Other
    };
    WoodBurner::burn_time_of(base, kind)
}

fn tick_wood_burner(game: &Game<'_>, entity: &mut ModBlockEntity) {
    let mut burner = WoodBurner {
        power: long(&entity.nbt, "power"),
        burn_time: int(&entity.nbt, "burnTime"),
        max_burn_time: int(&entity.nbt, "maxBurnTime"),
        // On until the machine has a screen with NTM's switch.
        is_on: true,
    };
    let fuel = entity.items[FUEL_SLOT]
        .as_ref()
        .map_or(0, |stack| fuel_burn_time(game, stack));
    if burner.tick(fuel) {
        let remainder = entity.items[FUEL_SLOT]
            .as_ref()
            .and_then(|stack| game.items().get(&stack.item))
            .and_then(|item| item.crafting_remainder.clone());
        let stack = entity.items[FUEL_SLOT].as_mut().unwrap();
        stack.count -= 1;
        if stack.count <= 0 {
            entity.items[FUEL_SLOT] = remainder.map(|item| ItemStack {
                item: item.as_str().into(),
                count: 1,
                ..entity.items[FUEL_SLOT].clone().unwrap()
            });
        }
    }
    entity.nbt.insert("power".into(), Tag::Long(burner.power));
    entity
        .nbt
        .insert("burnTime".into(), Tag::Int(burner.burn_time));
    entity
        .nbt
        .insert("maxBurnTime".into(), Tag::Int(burner.max_burn_time));
    // The furnace menu's flame: time left, total, then its arrow, unused.
    entity.data = vec![burner.burn_time, burner.max_burn_time, 0, 0];
}

struct Rules<'a, 'g>(&'a Game<'g>);

impl ShredderRules<ItemStack> for Rules<'_, '_> {
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
        self.0.items().get(&stack.item).map_or(64, |item| item.max_stack)
    }

    fn blade_damage(&self, stack: &ItemStack) -> Option<(u32, u32)> {
        if !stack.item.starts_with(BLADES_PREFIX) {
            return None;
        }
        let max = self.0.items().get(&stack.item).map_or(0, |item| item.max_damage);
        Some((stack.damage, max))
    }

    fn set_damage(&self, stack: &mut ItemStack, damage: u32) {
        stack.damage = damage;
    }

    fn result(&self, stack: &ItemStack) -> Option<ItemStack> {
        let game = self.0;
        let mut results = SHREDDER_RESULTS.lock().unwrap_or_else(|e| e.into_inner());
        results.get_or_insert_with(RecipeCache::default).get(&stack.item, || {
            let recipe = game
                .native_recipes_of_type(SHREDDER_RECIPE)
                .iter()
                .find(|recipe| {
                    game.native_recipe_ingredient(&recipe.json["ingredient"])
                        .is_some_and(|ingredient| ingredient.test(stack))
                })
                .and_then(|recipe| game.native_recipe_result(&recipe.json["result"]));
            recipe.or_else(|| game.native_recipe_result(&serde_json::json!({ "id": SCRAP })))
        })
    }
}

fn tick_shredder(game: &Game<'_>, entity: &mut ModBlockEntity) {
    let mut shredder = Shredder {
        power: long(&entity.nbt, "power"),
        progress: int(&entity.nbt, "progress"),
    };
    let rules = Rules(game);
    shredder.tick(&rules, &mut entity.items);
    entity.nbt.insert("power".into(), Tag::Long(shredder.power));
    entity
        .nbt
        .insert("progress".into(), Tag::Int(shredder.progress));
    let left = Shredder::gear(&rules, entity.items[Shredder::LEFT_BLADE].as_ref());
    let right = Shredder::gear(&rules, entity.items[Shredder::RIGHT_BLADE].as_ref());
    entity.data = vec![shredder.power as i32, shredder.progress, left, right];
}

pub(super) fn screen(block: &str, screen: &mut Option<NativeScreen>) {
    if block != SHREDDER || screen.is_some() {
        return;
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
    *screen = Some(NativeScreen {
        width: 176.0,
        height: 233.0,
        slots,
        inventory_x: 8.0,
        inventory_y: 151.0,
        own_labels: true,
        own_background: true,
    });
}

/// `GUIMachineShredder.drawGuiContainerBackgroundLayer` and `drawGuiContainerForegroundLayer`.
pub(super) fn screen_draw(
    game: &Game<'_>,
    block: &str,
    data: &[i32],
    batch: &mut HudBatch,
    origin: [f32; 2],
) {
    if block != SHREDDER {
        return;
    }
    let [power, progress, left, right] = *data else {
        return;
    };
    let mut part = |src: [f32; 2], dst: [f32; 2], size: [f32; 2]| {
        game.native_sprite(
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
    if let Some(title) = game.native_screen_title() {
        let x = 106.0 - game.native_text_width(&title) / 2.0;
        game.native_text(batch, &title, [origin[0] + x, origin[1] + 6.0], LABEL, false);
    }
    let inventory = game.translate("container.inventory", "Inventory");
    game.native_text(batch, &inventory, [origin[0] + 8.0, origin[1] + 139.0], LABEL, false);
}

pub(super) fn screen_tooltip(block: &str, data: &[i32], at: [f32; 2], lines: &mut Vec<String>) {
    if block != SHREDDER {
        return;
    }
    let [power, _, left, right] = *data else {
        return;
    };
    if (8.0..24.0).contains(&at[0]) && (18.0..106.0).contains(&at[1]) {
        lines.push(format!("{power}/{} HE", Shredder::MAX_POWER));
    } else if (43.0..97.0).contains(&at[0])
        && (71.0..89.0).contains(&at[1])
        && [left, right].iter().any(|gear| matches!(gear, 0 | 3))
    {
        lines.push("Error: Shredder blades are broken or missing!".to_owned());
    }
}

/// `ItemPowerNetTool`: right click on a cable or machine for its network, plus what this machine
/// holds, which the original shows on the machine's screen.
pub(super) fn use_item_on(game: &mut Game<'_>, stack: &ItemStack, pos: Pos, claimed: &mut bool) {
    if &*stack.item != NET_TOOL {
        return;
    }
    *claimed = true;
    let Some(player) = game.native_acting_player() else {
        return;
    };
    let mut lines = with_level(game.native_dimension(), |level| {
        let net = level
            .space
            .node_at(pos)
            .and_then(|node| level.space.net_of(node));
        match net.and_then(|net| Some((level.space.net_sizes(net)?, level.space.energy_tracker(net)?))) {
            Some(((links, providers, receivers), flow)) => vec![
                format!("Links: {links}"),
                format!("Providers: {providers}"),
                format!("Receivers: {receivers}"),
                format!("Last flow: {flow} HE/t"),
            ],
            None => vec!["Error: No network found!".to_owned()],
        }
    });
    let handlers = Handlers(game);
    let max = handlers.max_power(pos);
    if max > 0 {
        lines.push(format!("Power: {} / {max} HE", handlers.power(pos)));
    }
    for line in lines {
        game.native_tell(player.uuid, Text::literal(line));
    }
}
