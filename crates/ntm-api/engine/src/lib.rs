//! NTM through Horizon's public mod API only: the trait hooks and `Host`, no mixins. The logic is
//! in `ntm_core`, and the machines of its registry (`machines::MACHINES`) all go through the same
//! code here. Where the API has no way yet, a stub stands in, marked `API gap`.

use horizon_client::mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, BlockEntityContainerEvent, BlockEntityEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, ChunkLoadEvent, ChunkUnloadEvent, Compound,
    Flow, Holder, Hooks, Host, ItemStack, LevelTickPreEvent, MenuButtonEvent, MenuSlotRulesEvent,
    ModBlockEntity, ModContainer, NativeScreen, ScreenClickEvent, ScreenDrawEvent, ScreenEvent,
    ScreenTooltipEvent, ServerDataLoadedEvent, ServerTickEvent, Tag, Text, UseItemOnEvent,
    WorldLifecycleEvent, WorldPhase,
};
use ntm_core::battery::{
    self, Battery, BatteryPack, BatterySlot, ChargeLog, SOCKET_BLOCK, SOCKET_MENU, SOCKET_SCREEN,
    StorageMode, StorageSettings,
};
use ntm_core::energy::{EnergyHandlers, NodeId, Nodespace, Pos, Port, Priority, around};
use ntm_core::machines::screen::{self, Screen};
use ntm_core::machines::{self, FuelKind, MachineWorld, Nbt, RecipeCaches, Role, WoodBurner};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, Once};

const CABLE: &str = "ntm:red_cable";
const WOOD_BURNER: &str = "ntm:machine_wood_burner";
const NET_TOOL: &str = "ntm:power_net_tool";
const CUSTOM_DATA: &str = "minecraft:custom_data";
const SCRAP: &str = "ntm:scrap";
const SHREDDER_RECIPE: &str = "ntm:shredder";
const BLAST_RECIPE: &str = "ntm:blast_furnace";
const BLAST_FUEL: &str = "ntm:blast_furnace_fuel";
const PRESS_RECIPE: &str = "ntm:press";
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

/// What the network needs from a socket beside its power: the battery's capacity, the speeds its
/// mode allows and its priority. API gap: the network can't read the battery in the socket, so
/// the socket writes this each tick, unsaved, and keeps `POWER` as its battery's charge.
#[derive(Clone, Copy, Debug, Default)]
struct Storage {
    max: i64,
    receive: i64,
    provide: i64,
    priority: i32,
}

static STORAGE: AttachmentType<Storage> = AttachmentType {
    namespace: "ntm",
    name: "storage",
    default: Storage::default,
    codec: None,
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
    recipes: Mutex<RecipeCaches<ItemStack>>,
}

impl NtmHooks {
    fn with_level<R>(&self, dimension: &str, f: impl FnOnce(&mut Level) -> R) -> R {
        let mut levels = self.levels.lock().unwrap_or_else(|e| e.into_inner());
        f(levels.entry(dimension.to_owned()).or_default())
    }

    /// The recipes and tags were (re)built: what was found in them is stale.
    fn forget_recipes(&self) {
        self.recipes.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

fn chunk_of(pos: Pos) -> [i32; 2] {
    [pos[0] >> 4, pos[2] >> 4]
}

fn is_machine(block: &str) -> bool {
    block == WOOD_BURNER || block == SOCKET_BLOCK || machines::spec(block).is_some()
}

/// A battery pack as `IBatteryItem` sees it, its charge the long `charge` of its custom data, the
/// NBT tag NTM keeps it in.
fn pack_battery(stack: &ItemStack) -> Option<Battery> {
    let pack = BatteryPack::from_name(stack.item.strip_prefix(machines::PACK_PREFIX)?)?;
    let charge = stack
        .extra
        .as_ref()
        .and_then(|data| data.other.iter().find(|(key, _)| key == CUSTOM_DATA))
        .and_then(|(_, tag)| match tag {
            Tag::Compound(data) => match data.get(&"charge".into()) {
                Some(Tag::Long(charge)) => Some(*charge),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(0);
    Some(pack.battery(charge))
}

fn set_pack_charge(stack: &mut ItemStack, charge: i64) {
    let other = &mut stack.extra.get_or_insert_with(Default::default).other;
    match other.iter_mut().find(|(key, _)| key == CUSTOM_DATA) {
        Some((_, Tag::Compound(data))) => {
            data.insert("charge".into(), Tag::Long(charge));
        }
        Some((_, tag)) => *tag = Tag::Compound([("charge".into(), Tag::Long(charge))].into()),
        None => {
            // The components stay in key order.
            let at = other.partition_point(|(key, _)| key.as_str() < CUSTOM_DATA);
            other.insert(
                at,
                (CUSTOM_DATA.into(), Tag::Compound([("charge".into(), Tag::Long(charge))].into())),
            );
        }
    }
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

fn socket_settings(nbt: &Compound) -> StorageSettings {
    let get = |key: &str| match nbt.get(&key.into()) {
        Some(Tag::Short(n)) => Some(i32::from(*n)),
        Some(Tag::Byte(n)) => Some(i32::from(*n)),
        _ => None,
    };
    let default = StorageSettings::default();
    StorageSettings {
        red_low: get("redLow").map_or(default.red_low, StorageMode::from_index),
        red_high: get("redHigh").map_or(default.red_high, StorageMode::from_index),
        priority: get("priority").map_or(default.priority, Priority::from_ordinal),
    }
    .checked()
}

fn put_socket_settings(nbt: &mut Compound, settings: StorageSettings) {
    nbt.insert("redLow".into(), Tag::Short(settings.red_low.index() as i16));
    nbt.insert("redHigh".into(), Tag::Short(settings.red_high.index() as i16));
    nbt.insert("priority".into(), Tag::Byte(settings.priority.ordinal() as i8));
}

/// `TileEntityBatterySocket.updateEntity`, the network's part done. The network moved `POWER`;
/// the battery takes what it moved since the last tick wrote it (`synced`), then `POWER` and
/// `STORAGE` show the battery as it now is. A battery swapped between the two keeps the change.
fn tick_socket(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, mode: StorageMode) {
    let settings = socket_settings(&entity.nbt);
    put_socket_settings(&mut entity.nbt, settings);
    let moved = power(host, pos).wrapping_sub(long(&entity.nbt, "synced"));
    let mut battery = None;
    if let Some(Some(stack)) = entity.items.first_mut()
        && let Some(mut found) = pack_battery(stack)
    {
        found.charge = found.charge.wrapping_add(moved).clamp(0, found.max_charge);
        set_pack_charge(stack, found.charge);
        battery = Some(found);
    }
    let (charge, max) = battery.map_or((0, 0), |battery| (battery.charge, battery.max_charge));
    set_power(host, pos, charge);
    entity.nbt.insert("synced".into(), Tag::Long(charge));
    let _ = STORAGE.set(
        host,
        Holder::BlockEntity(pos),
        Storage {
            max,
            receive: battery::receiver_speed(mode, battery.as_ref()),
            provide: battery::provider_speed(mode, battery.as_ref()),
            priority: settings.priority.ordinal(),
        },
    );
    let mut log = ChargeLog::default();
    if let Some(Tag::LongArray(saved)) = entity.nbt.get(&"log".into())
        && saved.len() == log.log.len()
    {
        log.log.copy_from_slice(saved);
    }
    let delta = log.push(charge, charge);
    entity.nbt.insert("log".into(), Tag::LongArray(log.log.to_vec()));
    entity.data = battery::socket_data(charge, max, delta, settings);
}

fn max_power_of(block: &str) -> i64 {
    match block {
        WOOD_BURNER => WoodBurner::MAX_POWER,
        _ => machines::spec(block).map_or(0, |spec| spec.max_power),
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
        self.storage(pos)
            .map_or_else(|| max_power_of(&self.0.borrow().block(pos)), |storage| storage.max)
    }

    fn is_bad_link(&self, pos: Pos) -> bool {
        !is_machine(&self.0.borrow().block(pos))
    }

    fn receiver_speed(&self, pos: Pos) -> i64 {
        self.storage(pos).map_or_else(|| self.max_power(pos), |storage| storage.receive)
    }

    fn provider_speed(&self, pos: Pos) -> i64 {
        self.storage(pos).map_or_else(|| self.max_power(pos), |storage| storage.provide)
    }

    fn priority(&self, pos: Pos) -> Priority {
        self.storage(pos).map_or(Priority::Normal, |storage| Priority::from_ordinal(storage.priority))
    }
}

impl Handlers<'_> {
    /// A socket's `STORAGE`, none for anything else.
    fn storage(&self, pos: Pos) -> Option<Storage> {
        let mut host = self.0.borrow_mut();
        (host.block(pos) == SOCKET_BLOCK)
            .then(|| STORAGE.get(*host, Holder::BlockEntity(pos)).unwrap_or_default())
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

/// API gap: no item data in `Host`, so no `crafting_remainder`. Vanilla's one fuel with one.
fn remainder_of(item: &str) -> Option<ItemStack> {
    (item == "minecraft:lava_bucket").then(|| ItemStack {
        item: "minecraft:bucket".into(),
        count: 1,
        damage: 0,
        extra: None,
    })
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
            entity.items[FUEL_SLOT] = remainder_of(&stack.item);
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

/// A machine's saved fields in its block entity's NBT.
struct Fields<'a>(&'a mut Compound);

impl Nbt for Fields<'_> {
    fn int(&self, key: &str) -> i32 {
        int(self.0, key)
    }

    fn set_int(&mut self, key: &str, value: i32) {
        self.0.insert(key.into(), Tag::Int(value));
    }
}

/// The machine at `pos` and its world, as `ntm_core`'s machines see them. Its power is read from
/// `POWER` before the tick and written back after.
struct World<'a> {
    host: &'a dyn Host,
    pos: Pos,
    power: i64,
    recipes: &'a Mutex<RecipeCaches<ItemStack>>,
}

impl World<'_> {
    fn with_recipes<R>(&self, f: impl FnOnce(&mut RecipeCaches<ItemStack>) -> R) -> R {
        f(&mut self.recipes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn ingredient_fits(&self, json: &serde_json::Value, stack: &ItemStack) -> bool {
        self.host
            .recipe_ingredient(json)
            .is_some_and(|ingredient| ingredient.test(stack))
    }
}

impl BatterySlot<ItemStack> for World<'_> {
    fn battery(&self, stack: &ItemStack) -> Option<Battery> {
        pack_battery(stack)
    }

    fn set_charge(&self, stack: &mut ItemStack, charge: i64) {
        set_pack_charge(stack, charge);
    }
}

impl MachineWorld<ItemStack> for World<'_> {
    fn id<'s>(&self, stack: &'s ItemStack) -> &'s str {
        &stack.item
    }

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
        self.host.max_stack_size(&stack.item).unwrap_or(64)
    }

    fn damage(&self, stack: &ItemStack) -> (u32, u32) {
        // API gap: no item max damage in `Host`. The numbers ntm-content gives blades and stamps.
        let max = match stack.item.strip_prefix(machines::BLADES_PREFIX) {
            Some("steel") => 200,
            Some("titanium") => 350,
            Some(_) => 0,
            None => machines::press::stamp_durability(&stack.item),
        };
        (stack.damage, max)
    }

    fn set_damage(&self, stack: &mut ItemStack, damage: u32) {
        stack.damage = damage;
    }

    fn power(&self) -> i64 {
        self.power
    }

    fn set_power(&mut self, power: i64) {
        self.power = power;
    }

    fn neighbor(&self, offset: [i32; 3]) -> String {
        let [x, y, z] = self.pos;
        self.host.block([x + offset[0], y + offset[1], z + offset[2]])
    }

    fn burn_time(&self, stack: &ItemStack) -> i32 {
        base_burn_time(&stack.item)
    }

    fn remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        remainder_of(&stack.item)
    }

    fn shredder_result(&self, stack: &ItemStack) -> Option<ItemStack> {
        self.with_recipes(|recipes| {
            recipes.shredder.get(&stack.item, || {
                self.host
                    .recipes_of_type(SHREDDER_RECIPE)
                    .iter()
                    .find(|recipe| self.ingredient_fits(&recipe.json["ingredient"], stack))
                    .and_then(|recipe| self.host.recipe_result(&recipe.json["result"]))
                    .or_else(|| self.host.recipe_result(&serde_json::json!({ "id": SCRAP })))
            })
        })
    }

    /// `BlastFurnaceRecipes.getRequiredCounts`: the recipe's inputs either way round.
    fn blast_result(&self, a: &ItemStack, b: &ItemStack) -> Option<(u32, u32, ItemStack)> {
        self.with_recipes(|recipes| {
            recipes.blast.get(&format!("{}|{}", a.item, b.item), || {
                self.host.recipes_of_type(BLAST_RECIPE).iter().find_map(|recipe| {
                    let json = &recipe.json;
                    let count = |key: &str| json[key].as_u64().map_or(1, |n| n as u32);
                    let (need_a, need_b) = (count("input_a_count"), count("input_b_count"));
                    let out = self.host.recipe_result(&json["result"])?;
                    if self.ingredient_fits(&json["input_a"], a) && self.ingredient_fits(&json["input_b"], b) {
                        Some((need_a, need_b, out))
                    } else if self.ingredient_fits(&json["input_a"], b) && self.ingredient_fits(&json["input_b"], a) {
                        Some((need_b, need_a, out))
                    } else {
                        None
                    }
                })
            })
        })
    }

    fn blast_fuel(&self, stack: &ItemStack) -> i32 {
        self.with_recipes(|recipes| {
            recipes.blast_fuel.get(&stack.item, || {
                self.host
                    .recipes_of_type(BLAST_FUEL)
                    .iter()
                    .find(|recipe| self.ingredient_fits(&recipe.json["ingredient"], stack))
                    .and_then(|recipe| recipe.json["power"].as_i64())
                    .map_or(0, |power| power as i32)
            })
        })
    }

    fn press_result(&self, stamp: &str, input: &ItemStack) -> Option<ItemStack> {
        self.with_recipes(|recipes| {
            recipes.press.get(&format!("{stamp}|{}", input.item), || {
                self.host
                    .recipes_of_type(PRESS_RECIPE)
                    .iter()
                    .find(|recipe| {
                        recipe.json["stamp"].as_str() == Some(stamp)
                            && self.ingredient_fits(&recipe.json["ingredient"], input)
                    })
                    .and_then(|recipe| self.host.recipe_result(&recipe.json["result"]))
            })
        })
    }
}

/// The screen of a block, if it is one of NTM's.
fn screen_of(block: &str) -> Option<&'static Screen> {
    match block {
        SOCKET_BLOCK => Some(&SOCKET_SCREEN),
        _ => machines::spec(block).map(|spec| spec.screen),
    }
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
            // NTM lets automation take a battery out once it is full or empty; here it only goes in.
            SOCKET_BLOCK => Some(ModContainer {
                slots: 1,
                faces: std::array::from_fn(|_| vec![0]),
                inputs: vec![0],
                outputs: vec![],
                menu: Some(SOCKET_MENU.into()),
            }),
            block => machines::spec(block).map(|spec| ModContainer {
                slots: spec.slots.len(),
                faces: std::array::from_fn(|_| spec.inputs.iter().chain(spec.outputs).copied().collect()),
                inputs: spec.inputs.to_vec(),
                outputs: spec.outputs.to_vec(),
                menu: Some(spec.menu.into()),
            }),
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
            return;
        }
        if !is_machine(&id) {
            return;
        }
        // API gap: no redstone signal in `Host` (`native_neighbor_signal`), so a socket keeps to
        // its mode without a signal.
        let mode = (id == SOCKET_BLOCK).then(|| socket_settings(&event.entity.nbt).mode(false));
        let role = match id.as_str() {
            WOOD_BURNER => Role::Provider,
            _ => machines::spec(&id).map_or(Role::None, |spec| spec.role),
        };
        if mode.is_some() || role != Role::None {
            self.with_level(host.dimension(), |level| {
                let Level { space, ports, .. } = level;
                let port = ports
                    .entry(pos)
                    .or_insert_with(|| Port::new(vec![pos], around(pos)));
                port.update(space, pos);
                match (mode, role) {
                    (Some(mode), _) => port.join(space, pos, mode.provides(), mode.receives()),
                    (None, Role::Provider) => port.check_provide(space, pos),
                    _ => port.check_subscribe(space, pos),
                }
            });
        }
        match mode {
            Some(mode) => tick_socket(host, pos, event.entity, mode),
            None if id == WOOD_BURNER => tick_wood_burner(host, pos, event.entity),
            None => {
                let entity = &mut *event.entity;
                let power = if role == Role::None { 0 } else { power(host, pos) };
                let mut world = World { host: &*host, pos, power, recipes: &self.recipes };
                // The slots are lent out for the tick: the NBT is borrowed beside them.
                let mut items = std::mem::take(&mut entity.items);
                let data = machines::tick(&id, &mut world, &mut Fields(&mut entity.nbt), &mut items);
                entity.items = items;
                let power = world.power;
                if let Some(data) = data {
                    if role != Role::None {
                        set_power(host, pos, power);
                    }
                    entity.data = data;
                }
            }
        }
    }

    /// `TileEntityBatteryBase.receiveControl`: the socket's buttons, on the server.
    fn menu_button(&self, _host: &mut dyn Host, event: &mut MenuButtonEvent<'_>) {
        if event.entity.id != SOCKET_BLOCK {
            return;
        }
        let mut settings = socket_settings(&event.entity.nbt);
        if settings.press(event.id) {
            put_socket_settings(&mut event.entity.nbt, settings);
        }
    }

    fn screen_click(&self, _host: &dyn Host, event: &mut ScreenClickEvent<'_>) -> Option<i32> {
        screen::click(screen_of(event.block)?, event.at)
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
        let max = match host.block(pos).as_str() {
            SOCKET_BLOCK => STORAGE.get(host, Holder::BlockEntity(pos)).unwrap_or_default().max,
            block => max_power_of(block),
        };
        if max > 0 {
            lines.push(format!("Power: {} / {max} HE", power(host, pos)));
        }
        for line in lines {
            host.tell(player.uuid, Text::literal(line));
        }
        Flow::Cancel
    }

    fn menu_slot_rules(&self, event: &mut MenuSlotRulesEvent<'_>) {
        if let Some(may_place) = machines::may_place(event.menu, event.slot, &event.stack.item) {
            event.may_place = may_place;
        }
    }

    fn screen(&self, _host: &dyn Host, event: &mut ScreenEvent<'_>) -> Option<NativeScreen> {
        let s = screen_of(event.block)?;
        Some(NativeScreen {
            width: s.size[0],
            height: s.size[1],
            slots: s.slots.to_vec(),
            inventory_x: s.inventory[0],
            inventory_y: s.inventory[1],
            own_labels: true,
            own_background: true,
        })
    }

    /// The machine's GUI as its screen describes it: the pieces `ntm_core` works out, then the
    /// labels.
    fn screen_draw(&self, host: &dyn Host, event: &mut ScreenDrawEvent<'_>) {
        let Some(s) = screen_of(event.block) else {
            return;
        };
        let origin = event.origin;
        let batch = &mut *event.batch;
        for blit in screen::blits(s, event.data) {
            let d = blit.dst;
            host.sprite(batch, s.texture, blit.src, [origin[0] + d[0], origin[1] + d[1], d[2], d[3]]);
        }
        if let Some(title) = host.screen_title() {
            // API gap: no text width in `Host`. Most of the font's glyphs are 6 pixels wide.
            let x = s.title.x - title.chars().count() as f32 * 3.0;
            host.text(batch, &title, [origin[0] + x, origin[1] + s.title.y], s.title.color, false);
        }
        // API gap: no translation in `Host`.
        let [x, y] = s.inventory_label;
        host.text(batch, "Inventory", [origin[0] + x, origin[1] + y], screen::DARK, false);
    }

    fn screen_tooltip(&self, _host: &dyn Host, event: &mut ScreenTooltipEvent<'_>) {
        if let Some(s) = screen_of(event.block) {
            event.lines.extend(screen::tooltip(s, event.data, event.at));
        }
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(NtmHooks::default())
}
