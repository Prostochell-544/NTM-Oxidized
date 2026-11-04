// The game's side of NTM's power network and machines, added as `game::ntm_direct`. The helpers
// beside this file call it from the hook points; the logic is in `ntm_core`, and the machines of
// its registry (`machines::MACHINES`) all go through the same code here.

use super::Game;
use crate::items::ItemStack;
use crate::text::Text;
use horizon_client_render::chunk::block_entity::{ModBlockEntity, ModContainer};
use horizon_world::nbt::{Compound, Tag};
use ntm_core::battery::{
    self, Battery, BatteryPack, BatterySlot, ChargeLog, SOCKET_BLOCK, SOCKET_MENU, SOCKET_SCREEN,
    StorageMode, StorageSettings,
};
use ntm_core::energy::{EnergyHandlers, NodeId, Nodespace, Pos, Port, Priority, around};
use crate::mod_api::{HudBatch, NativeScreen};
use ntm_core::machines::screen::{self, Screen};
use ntm_core::machines::{self, FuelKind, MachineWorld, Nbt, RecipeCaches, Role, WoodBurner};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

const CABLE: &str = "ntm:red_cable";
const WOOD_BURNER: &str = "ntm:machine_wood_burner";
const NET_TOOL: &str = "ntm:power_net_tool";
const SCRAP: &str = "ntm:scrap";
const SHREDDER_RECIPE: &str = "ntm:shredder";
const BLAST_RECIPE: &str = "ntm:blast_furnace";
const BLAST_FUEL: &str = "ntm:blast_furnace_fuel";
const PRESS_RECIPE: &str = "ntm:press";
const CUSTOM_DATA: &str = "minecraft:custom_data";
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
static RECIPES: Mutex<Option<RecipeCaches<ItemStack>>> = Mutex::new(None);

fn with_level<R>(dimension: &str, f: impl FnOnce(&mut Level) -> R) -> R {
    let mut levels = LEVELS.lock().unwrap_or_else(|e| e.into_inner());
    let levels = levels.get_or_insert_with(HashMap::new);
    f(levels.entry(dimension.to_owned()).or_default())
}

fn with_recipes<R>(f: impl FnOnce(&mut RecipeCaches<ItemStack>) -> R) -> R {
    let mut recipes = RECIPES.lock().unwrap_or_else(|e| e.into_inner());
    f(recipes.get_or_insert_with(RecipeCaches::default))
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

/// The battery in a socket's slot.
fn socket_battery(entity: &ModBlockEntity) -> Option<Battery> {
    entity.items.first()?.as_ref().and_then(pack_battery)
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

/// The mode its last tick found (`getRelevantMode(true)`), which the network reads.
fn socket_mode(nbt: &Compound) -> StorageMode {
    StorageMode::from_index(int(nbt, "mode"))
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
        _ => machines::spec(id).map_or(0, |spec| spec.max_power),
    }
}

/// The machines of the level ticking, as the power network reads and changes them. A socket's
/// power is the charge of the battery in it.
struct Handlers<'a, 'g>(&'a mut Game<'g>);

impl Handlers<'_, '_> {
    fn with_entity<R>(&self, pos: Pos, f: impl FnOnce(&ModBlockEntity) -> R) -> Option<R> {
        match self.0.world.block_entities.get(&(pos[0], pos[1], pos[2]))? {
            crate::game::BlockEntity::Mod(m) => Some(f(m)),
            _ => None,
        }
    }

    fn with_socket<R: Default>(&self, pos: Pos, f: impl FnOnce(&ModBlockEntity) -> R) -> Option<R> {
        (self.0.native_block(pos) == SOCKET_BLOCK).then(|| self.with_entity(pos, f).unwrap_or_default())
    }
}

impl EnergyHandlers<Pos> for Handlers<'_, '_> {
    fn power(&self, pos: Pos) -> i64 {
        self.with_entity(pos, |m| match m.id.as_str() {
            SOCKET_BLOCK => socket_battery(m).map_or(0, |battery| battery.charge),
            _ => long(&m.nbt, "power"),
        })
        .unwrap_or(0)
    }

    fn set_power(&mut self, pos: Pos, power: i64) {
        self.0.native_with_mod_block_entity(pos, |entity| {
            if entity.id != SOCKET_BLOCK {
                entity.nbt.insert("power".into(), Tag::Long(power));
            } else if let Some(Some(stack)) = entity.items.first_mut()
                && pack_battery(stack).is_some()
            {
                set_pack_charge(stack, power);
            }
        });
    }

    fn max_power(&self, pos: Pos) -> i64 {
        self.with_socket(pos, |m| socket_battery(m).map_or(0, |battery| battery.max_charge))
            .unwrap_or_else(|| max_power_of(&self.0.native_block(pos)))
    }

    fn is_bad_link(&self, pos: Pos) -> bool {
        !is_machine(&self.0.native_block(pos))
    }

    fn receiver_speed(&self, pos: Pos) -> i64 {
        self.with_socket(pos, |m| battery::receiver_speed(socket_mode(&m.nbt), socket_battery(m).as_ref()))
            .unwrap_or_else(|| self.max_power(pos))
    }

    fn provider_speed(&self, pos: Pos) -> i64 {
        self.with_socket(pos, |m| battery::provider_speed(socket_mode(&m.nbt), socket_battery(m).as_ref()))
            .unwrap_or_else(|| self.max_power(pos))
    }

    fn priority(&self, pos: Pos) -> Priority {
        self.with_entity(pos, |m| match m.id.as_str() {
            SOCKET_BLOCK => socket_settings(&m.nbt).priority,
            _ => Priority::Normal,
        })
        .unwrap_or(Priority::Normal)
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
    *RECIPES.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
    if container.is_some() {
        return;
    }
    if block == WOOD_BURNER {
        // A furnace's menu until the machine has a screen of its own: only its fuel slot works.
        *container = Some(ModContainer {
            slots: 3,
            faces: std::array::from_fn(|_| vec![FUEL_SLOT]),
            inputs: vec![FUEL_SLOT],
            outputs: vec![],
            menu: Some("minecraft:furnace".into()),
        });
    } else if block == SOCKET_BLOCK {
        // NTM lets automation take a battery out once it is full or empty; here it only goes in.
        *container = Some(ModContainer {
            slots: 1,
            faces: std::array::from_fn(|_| vec![0]),
            inputs: vec![0],
            outputs: vec![],
            menu: Some(SOCKET_MENU.into()),
        });
    } else if let Some(spec) = machines::spec(block) {
        *container = Some(ModContainer {
            slots: spec.slots.len(),
            faces: std::array::from_fn(|_| spec.inputs.iter().chain(spec.outputs).copied().collect()),
            inputs: spec.inputs.to_vec(),
            outputs: spec.outputs.to_vec(),
            menu: Some(spec.menu.into()),
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
        return;
    }
    if !is_machine(&id) {
        return;
    }
    let dimension = game.native_dimension().to_owned();
    // `getRelevantMode`: a signal reaching the socket switches it to its other mode.
    let mode = (id == SOCKET_BLOCK).then(|| {
        let powered = game.native_neighbor_signal(&dimension, pos) > 0;
        socket_settings(&entity.nbt).mode(powered)
    });
    let role = match id.as_str() {
        WOOD_BURNER => Role::Provider,
        _ => machines::spec(&id).map_or(Role::None, |spec| spec.role),
    };
    if mode.is_some() || role != Role::None {
        with_level(&dimension, |level| {
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
        Some(mode) => tick_socket(entity, mode),
        None if id == WOOD_BURNER => tick_wood_burner(game, entity),
        None => {
            let mut world = World { game: &*game, pos, power: long(&entity.nbt, "power") };
            // The slots are lent out for the tick: the NBT is borrowed beside them.
            let mut items = std::mem::take(&mut entity.items);
            let data = machines::tick(&id, &mut world, &mut Fields(&mut entity.nbt), &mut items);
            entity.items = items;
            if let Some(data) = data {
                if role != Role::None {
                    entity.nbt.insert("power".into(), Tag::Long(world.power));
                }
                entity.data = data;
            }
        }
    }
}

/// `TileEntityBatterySocket.updateEntity`, the network's part done: the mode the network reads
/// next, the last second's change, and what its screen shows.
fn tick_socket(entity: &mut ModBlockEntity, mode: StorageMode) {
    let settings = socket_settings(&entity.nbt);
    put_socket_settings(&mut entity.nbt, settings);
    entity.nbt.insert("mode".into(), Tag::Int(mode.index()));
    let battery = socket_battery(entity);
    let (charge, max) = battery.map_or((0, 0), |battery| (battery.charge, battery.max_charge));
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

/// The machine at `pos` and its world, as `ntm_core`'s machines see them.
struct World<'a, 'g> {
    game: &'a Game<'g>,
    pos: Pos,
    power: i64,
}

impl World<'_, '_> {
    fn ingredient_fits(&self, json: &serde_json::Value, stack: &ItemStack) -> bool {
        self.game
            .native_recipe_ingredient(json)
            .is_some_and(|ingredient| ingredient.test(stack))
    }

    fn result(&self, json: &serde_json::Value) -> Option<ItemStack> {
        self.game.native_recipe_result(json)
    }
}

impl BatterySlot<ItemStack> for World<'_, '_> {
    fn battery(&self, stack: &ItemStack) -> Option<Battery> {
        pack_battery(stack)
    }

    fn set_charge(&self, stack: &mut ItemStack, charge: i64) {
        set_pack_charge(stack, charge);
    }
}

impl MachineWorld<ItemStack> for World<'_, '_> {
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
        self.game.items().get(&stack.item).map_or(64, |item| item.max_stack)
    }

    fn damage(&self, stack: &ItemStack) -> (u32, u32) {
        let max = self.game.items().get(&stack.item).map_or(0, |item| item.max_damage);
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
        self.game.native_block([x + offset[0], y + offset[1], z + offset[2]])
    }

    fn burn_time(&self, stack: &ItemStack) -> i32 {
        self.game.res.providers().burn_time(self.game.items(), stack, false)
    }

    fn remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        let item = self.game.items().get(&stack.item)?.crafting_remainder.clone()?;
        Some(ItemStack { item: item.as_str().into(), count: 1, damage: 0, extra: None })
    }

    fn shredder_result(&self, stack: &ItemStack) -> Option<ItemStack> {
        with_recipes(|recipes| {
            recipes.shredder.get(&stack.item, || {
                self.game
                    .native_recipes_of_type(SHREDDER_RECIPE)
                    .iter()
                    .find(|recipe| self.ingredient_fits(&recipe.json["ingredient"], stack))
                    .and_then(|recipe| self.result(&recipe.json["result"]))
                    .or_else(|| self.result(&serde_json::json!({ "id": SCRAP })))
            })
        })
    }

    /// `BlastFurnaceRecipes.getRequiredCounts`: the recipe's inputs either way round.
    fn blast_result(&self, a: &ItemStack, b: &ItemStack) -> Option<(u32, u32, ItemStack)> {
        with_recipes(|recipes| {
            recipes.blast.get(&format!("{}|{}", a.item, b.item), || {
                self.game.native_recipes_of_type(BLAST_RECIPE).iter().find_map(|recipe| {
                    let json = &recipe.json;
                    let count = |key: &str| json[key].as_u64().map_or(1, |n| n as u32);
                    let (need_a, need_b) = (count("input_a_count"), count("input_b_count"));
                    let out = self.result(&json["result"])?;
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
        with_recipes(|recipes| {
            recipes.blast_fuel.get(&stack.item, || {
                self.game
                    .native_recipes_of_type(BLAST_FUEL)
                    .iter()
                    .find(|recipe| self.ingredient_fits(&recipe.json["ingredient"], stack))
                    .and_then(|recipe| recipe.json["power"].as_i64())
                    .map_or(0, |power| power as i32)
            })
        })
    }

    fn press_result(&self, stamp: &str, input: &ItemStack) -> Option<ItemStack> {
        with_recipes(|recipes| {
            recipes.press.get(&format!("{stamp}|{}", input.item), || {
                self.game
                    .native_recipes_of_type(PRESS_RECIPE)
                    .iter()
                    .find(|recipe| {
                        recipe.json["stamp"].as_str() == Some(stamp)
                            && self.ingredient_fits(&recipe.json["ingredient"], input)
                    })
                    .and_then(|recipe| self.result(&recipe.json["result"]))
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

pub(super) fn screen(block: &str, screen: &mut Option<NativeScreen>) {
    if screen.is_some() {
        return;
    }
    if let Some(s) = screen_of(block) {
        *screen = Some(NativeScreen {
            width: s.size[0],
            height: s.size[1],
            slots: s.slots.to_vec(),
            inventory_x: s.inventory[0],
            inventory_y: s.inventory[1],
            own_labels: true,
            own_background: true,
        });
    }
}

/// The machine's GUI as its screen describes it: the pieces `ntm_core` works out, then the labels.
pub(super) fn screen_draw(
    game: &Game<'_>,
    block: &str,
    data: &[i32],
    batch: &mut HudBatch,
    origin: [f32; 2],
) {
    let Some(s) = screen_of(block) else {
        return;
    };
    for blit in screen::blits(s, data) {
        let d = blit.dst;
        game.native_sprite(batch, s.texture, blit.src, [origin[0] + d[0], origin[1] + d[1], d[2], d[3]]);
    }
    if let Some(title) = game.native_screen_title() {
        let x = s.title.x - game.native_text_width(&title) / 2.0;
        game.native_text(batch, &title, [origin[0] + x, origin[1] + s.title.y], s.title.color, false);
    }
    let inventory = game.translate("container.inventory", "Inventory");
    let [x, y] = s.inventory_label;
    game.native_text(batch, &inventory, [origin[0] + x, origin[1] + y], screen::DARK, false);
}

pub(super) fn screen_click(block: &str, at: [f32; 2], button: &mut Option<i32>) {
    if button.is_none()
        && let Some(s) = screen_of(block)
    {
        *button = screen::click(s, at);
    }
}

/// `TileEntityBatteryBase.receiveControl`: the socket's buttons, on the server.
pub(super) fn menu_button(entity: &mut ModBlockEntity, id: i32) {
    if entity.id != SOCKET_BLOCK {
        return;
    }
    let mut settings = socket_settings(&entity.nbt);
    if settings.press(id) {
        put_socket_settings(&mut entity.nbt, settings);
    }
}

pub(super) fn screen_tooltip(block: &str, data: &[i32], at: [f32; 2], lines: &mut Vec<String>) {
    if let Some(s) = screen_of(block) {
        lines.extend(screen::tooltip(s, data, at));
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
