//! Machines, without the world around them, and what they share: how a machine sees its stacks and
//! the world (`MachineWorld`), slot helpers, recipe caches, saved fields (`Nbt`), the registry the
//! branches build blocks, containers and screens from (`MACHINES`), and one tick for all (`tick`).
//! Each machine's own logic is in its own module.

pub mod difurnace;
pub mod press;
pub mod screen;
pub mod shredder;
#[cfg(test)]
mod testing;
pub mod wood_burner;

pub use difurnace::DiFurnace;
pub use press::Press;
pub use shredder::Shredder;
pub use wood_burner::{FuelKind, WoodBurner};

use crate::battery::BatterySlot;
use screen::Screen;
use std::collections::HashMap;

pub const BLADES_PREFIX: &str = "ntm:blades_";
pub const PACK_PREFIX: &str = "ntm:battery_pack_";
pub const STAMP_PREFIX: &str = "ntm:stamp_";

/// How a machine sees its stacks and the world around it, whatever they are. A branch answers for
/// the block being ticked; the lookups a machine doesn't use keep their defaults.
pub trait MachineWorld<S>: BatterySlot<S> {
    /// The item's id, `ntm:ingot_steel`.
    fn id<'s>(&self, stack: &'s S) -> &'s str;
    fn same_item(&self, a: &S, b: &S) -> bool;
    fn count(&self, stack: &S) -> u32;
    fn set_count(&self, stack: &mut S, count: u32);
    fn max_stack(&self, stack: &S) -> u32;
    /// `(damage, max damage)`; max damage 0 never wears out.
    fn damage(&self, stack: &S) -> (u32, u32);
    fn set_damage(&self, stack: &mut S, damage: u32);

    /// This machine's power, HE.
    fn power(&self) -> i64 {
        0
    }
    fn set_power(&mut self, _power: i64) {}

    /// The block beside this one, by offset: `[0, 1, 0]` is the one above.
    fn neighbor(&self, _offset: [i32; 3]) -> String {
        String::new()
    }
    /// A furnace's burn time for the item, 0 for what doesn't burn.
    fn burn_time(&self, _stack: &S) -> i32 {
        0
    }
    /// What an item leaves behind when used up (`getContainerItem`): a bucket after lava.
    fn remainder(&self, _stack: &S) -> Option<S> {
        None
    }

    /// A blade's `(damage, max damage)`, none for anything else.
    fn blade_damage(&self, stack: &S) -> Option<(u32, u32)> {
        self.id(stack).starts_with(BLADES_PREFIX).then(|| self.damage(stack))
    }
    /// A stamp's type, as press recipes name it.
    fn stamp(&self, stack: &S) -> Option<&'static str> {
        press::stamp_type(self.id(stack))
    }

    /// What `stack` shreds into (`ntm:shredder` recipes; scrap without one).
    fn shredder_result(&self, _stack: &S) -> Option<S> {
        None
    }
    /// The blast furnace's recipe for these two inputs, either way round: how many of each it takes
    /// and what it makes (`ntm:blast_furnace`).
    fn blast_result(&self, _a: &S, _b: &S) -> Option<(u32, u32, S)> {
        None
    }
    /// The blast furnace's fuel value of an item (`ntm:blast_furnace_fuel`).
    fn blast_fuel(&self, _stack: &S) -> i32 {
        0
    }
    /// What the press makes of `input` under a stamp of this type (`ntm:press`).
    fn press_result(&self, _stamp: &str, _input: &S) -> Option<S> {
        None
    }
}

/// Whether `out` goes whole into `slot`: an empty slot, or the same item with room.
pub fn fits<S>(world: &impl MachineWorld<S>, slot: &Option<S>, out: &S) -> bool {
    match slot {
        None => true,
        Some(have) => {
            world.same_item(have, out) && world.count(have) + world.count(out) <= world.max_stack(have)
        }
    }
}

/// Puts `out` into `slot`, which [`fits`] it.
pub fn put<S: Clone>(world: &impl MachineWorld<S>, slot: &mut Option<S>, out: &S) {
    match slot {
        Some(have) => {
            let count = world.count(have) + world.count(out);
            world.set_count(have, count);
        }
        None => *slot = Some(out.clone()),
    }
}

/// `decrStackSize`: takes `n` from the slot, emptying it at zero.
pub fn take<S>(world: &impl MachineWorld<S>, slot: &mut Option<S>, n: u32) {
    if let Some(stack) = slot {
        let left = world.count(stack).saturating_sub(n);
        if left == 0 {
            *slot = None;
        } else {
            world.set_count(stack, left);
        }
    }
}

/// Uses up one fuel item: the last one leaves what it leaves behind (a bucket after lava).
pub fn burn_one<S>(world: &impl MachineWorld<S>, slot: &mut Option<S>) {
    let Some(stack) = slot else {
        return;
    };
    if world.count(stack) == 1
        && let Some(left) = world.remainder(stack)
    {
        *slot = Some(left);
    } else {
        take(world, slot, 1);
    }
}

/// A machine's own saved fields, in its block entity's NBT.
pub trait Nbt {
    fn int(&self, key: &str) -> i32;
    fn set_int(&mut self, key: &str, value: i32);
}

/// Results found by input, as NTM keeps its machine recipes in maps keyed by what goes in: a
/// lookup is found once, then answered from here until the data reloads and `clear` empties it.
#[derive(Clone, Debug)]
pub struct RecipeCache<V> {
    results: HashMap<String, V>,
}

impl<V> Default for RecipeCache<V> {
    fn default() -> Self {
        Self { results: HashMap::new() }
    }
}

impl<V: Clone> RecipeCache<V> {
    /// The result for `key`, found with `find` the first time it is asked.
    pub fn get(&mut self, key: &str, find: impl FnOnce() -> V) -> V {
        if let Some(result) = self.results.get(key) {
            return result.clone();
        }
        let result = find();
        self.results.insert(key.to_owned(), result.clone());
        result
    }

    pub fn clear(&mut self) {
        self.results.clear();
    }
}

/// Every machine's recipe cache, emptied together when the data reloads.
#[derive(Clone, Debug)]
pub struct RecipeCaches<S> {
    pub shredder: RecipeCache<Option<S>>,
    pub blast: RecipeCache<Option<(u32, u32, S)>>,
    pub blast_fuel: RecipeCache<i32>,
    pub press: RecipeCache<Option<S>>,
}

impl<S> Default for RecipeCaches<S> {
    fn default() -> Self {
        Self {
            shredder: RecipeCache::default(),
            blast: RecipeCache::default(),
            blast_fuel: RecipeCache::default(),
            press: RecipeCache::default(),
        }
    }
}

impl<S: Clone> RecipeCaches<S> {
    pub fn clear(&mut self) {
        self.shredder.clear();
        self.blast.clear();
        self.blast_fuel.clear();
        self.press.clear();
    }
}

/// How a machine is on the power network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Not on it.
    None,
    Provider,
    Receiver,
}

/// What a player may put in a slot from the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// Anything but blades, batteries and stamps.
    Input,
    Fuel,
    Blade,
    Battery,
    Stamp,
    /// Only taken from.
    Output,
    /// Anything.
    Storage,
}

/// What the branches build a machine's block, container and screen from.
#[derive(Clone, Copy, Debug)]
pub struct MachineSpec {
    pub block: &'static str,
    /// Its menu type, declared in the content mod's `horizon/menu/`.
    pub menu: &'static str,
    pub slots: &'static [SlotKind],
    /// The slots hoppers fill and empty.
    pub inputs: &'static [usize],
    pub outputs: &'static [usize],
    pub role: Role,
    pub max_power: i64,
    pub screen: &'static Screen,
}

pub const MACHINES: &[MachineSpec] = &[shredder::SPEC, difurnace::SPEC, press::SPEC];

pub fn spec(block: &str) -> Option<&'static MachineSpec> {
    MACHINES.iter().find(|spec| spec.block == block)
}

/// What a player may put in `slot` of menu `menu`; none for a menu not NTM's.
pub fn may_place(menu: &str, slot: usize, item: &str) -> Option<bool> {
    let special = |item: &str| {
        [BLADES_PREFIX, PACK_PREFIX, STAMP_PREFIX].iter().any(|prefix| item.starts_with(prefix))
    };
    if menu == crate::battery::SOCKET_MENU {
        return Some(item.starts_with(PACK_PREFIX));
    }
    let spec = MACHINES.iter().find(|spec| spec.menu == menu)?;
    Some(match spec.slots.get(slot) {
        Some(SlotKind::Input | SlotKind::Fuel) => !special(item),
        Some(SlotKind::Blade) => item.starts_with(BLADES_PREFIX),
        Some(SlotKind::Battery) => item.starts_with(PACK_PREFIX),
        Some(SlotKind::Stamp) => item.starts_with(STAMP_PREFIX),
        Some(SlotKind::Storage) => true,
        Some(SlotKind::Output) | None => false,
    })
}

/// One tick of the machine `block`: its saved fields from `nbt`, its power from `world`, both put
/// back after. Answers its menu data, none for a block that isn't one of [`MACHINES`].
pub fn tick<S: Clone, W: MachineWorld<S>>(
    block: &str,
    world: &mut W,
    nbt: &mut impl Nbt,
    slots: &mut [Option<S>],
) -> Option<Vec<i32>> {
    match block {
        shredder::BLOCK => {
            let mut shredder = Shredder { power: world.power(), progress: nbt.int("progress") };
            shredder.tick(&*world, slots);
            world.set_power(shredder.power);
            nbt.set_int("progress", shredder.progress);
            Some(shredder.data(&*world, slots))
        }
        difurnace::BLOCK => {
            let mut furnace = DiFurnace::load(nbt);
            furnace.tick(&*world, slots);
            furnace.save(nbt);
            Some(furnace.data(&*world, slots))
        }
        press::BLOCK => {
            let mut press = Press::load(nbt);
            press.tick(&*world, slots);
            press.save(nbt);
            Some(press.data())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Fields, World, stack};
    use super::*;

    #[test]
    fn one_tick_for_all_keeps_fields_in_nbt_and_power_in_the_world() {
        let (mut w, mut nbt) = (World { power: 1_000, ..World::default() }, Fields::default());
        let mut slots = vec![stack("iron", 1), stack("coal", 1), stack("coal", 1), None];
        let data = tick(difurnace::BLOCK, &mut w, &mut nbt, &mut slots).unwrap();
        assert_eq!((nbt.int("powerTime"), nbt.int("cookTime")), (199, 1));
        assert_eq!(data, vec![1, 199, 1]);

        let mut slots = vec![None; Shredder::SLOTS];
        let data = tick(shredder::BLOCK, &mut w, &mut nbt, &mut slots).unwrap();
        assert_eq!((w.power, data[0]), (1_000, 1_000));
        assert!(tick("minecraft:stone", &mut w, &mut nbt, &mut slots).is_none());
    }

    #[test]
    fn slots_take_what_their_kind_allows() {
        assert_eq!(may_place("ntm:shredder", 0, "minecraft:cobblestone"), Some(true));
        assert_eq!(may_place("ntm:shredder", 0, "ntm:blades_steel"), Some(false));
        assert_eq!(may_place("ntm:shredder", 27, "ntm:blades_steel"), Some(true));
        assert_eq!(may_place("ntm:shredder", 29, "ntm:battery_pack_battery_lead"), Some(true));
        assert_eq!(may_place("ntm:press", 1, "ntm:stamp_iron_plate"), Some(true));
        assert_eq!(may_place("ntm:press", 2, "ntm:stamp_iron_plate"), Some(false));
        assert_eq!(may_place("ntm:press", 3, "ntm:plate_iron"), Some(false));
        assert_eq!(may_place("ntm:battery_socket", 0, "ntm:battery_pack_battery_lead"), Some(true));
        assert_eq!(may_place("minecraft:furnace", 0, "minecraft:coal"), None);
    }
}
