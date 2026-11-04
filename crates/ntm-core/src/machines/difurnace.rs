//! `TileEntityDiFurnace`, the blast furnace (CE's, with recipes that take more than one of an input)
//! and `GUIDiFurnace`. The input's container items (CE puts them in the output) are left out.

use super::screen::{DARK, Fill, Label, Part, Screen, Value};
use super::{MachineSpec, MachineWorld, Nbt, Role, SlotKind};

pub const BLOCK: &str = "ntm:machine_difurnace_off";
/// Above the furnace it triples the speed on the same fuel.
pub const EXTENSION: &str = "ntm:machine_difurnace_ext";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiFurnace {
    pub progress: i32,
    pub fuel: i32,
}

impl DiFurnace {
    pub const MAX_FUEL: i32 = 12_800;
    pub const PROCESSING_SPEED: i32 = 400;
    pub const INPUT_A: usize = 0;
    pub const INPUT_B: usize = 1;
    pub const FUEL: usize = 2;
    pub const OUTPUT: usize = 3;

    pub fn load(nbt: &impl Nbt) -> DiFurnace {
        DiFurnace { progress: nbt.int("cookTime"), fuel: nbt.int("powerTime") }
    }

    pub fn save(&self, nbt: &mut impl Nbt) {
        nbt.set_int("cookTime", self.progress);
        nbt.set_int("powerTime", self.fuel);
    }

    /// The recipe the inputs make, with enough of each and room for the result.
    fn recipe<S>(&self, world: &impl MachineWorld<S>, slots: &[Option<S>]) -> Option<(u32, u32, S)> {
        let (Some(a), Some(b)) = (&slots[Self::INPUT_A], &slots[Self::INPUT_B]) else {
            return None;
        };
        if self.fuel <= 0 {
            return None;
        }
        let (need_a, need_b, out) = world.blast_result(a, b)?;
        (world.count(a) >= need_a && world.count(b) >= need_b && super::fits(world, &slots[Self::OUTPUT], &out))
            .then_some((need_a, need_b, out))
    }

    pub fn tick<S: Clone>(&mut self, world: &impl MachineWorld<S>, slots: &mut [Option<S>]) {
        let extension = world.neighbor([0, 1, 0]) == EXTENSION;
        if let Some(fuel) = &slots[Self::FUEL] {
            let power = world.blast_fuel(fuel);
            if power > 0 && self.fuel <= Self::MAX_FUEL - power {
                self.fuel += power;
                super::burn_one(world, &mut slots[Self::FUEL]);
            }
        }
        if self.recipe(world, slots).is_some() {
            self.fuel -= 1;
            self.progress += if extension { 3 } else { 1 };
            if self.progress >= Self::PROCESSING_SPEED {
                self.progress -= Self::PROCESSING_SPEED;
                if let Some((need_a, need_b, out)) = self.recipe(world, slots) {
                    super::put(world, &mut slots[Self::OUTPUT], &out);
                    super::take(world, &mut slots[Self::INPUT_A], need_a);
                    super::take(world, &mut slots[Self::INPUT_B], need_b);
                }
            }
            self.fuel = self.fuel.max(0);
        } else {
            self.progress = 0;
        }
    }

    /// The menu data [`SCREEN`] reads: progress, fuel, and whether the fire shows.
    pub fn data<S>(&self, world: &impl MachineWorld<S>, slots: &[Option<S>]) -> Vec<i32> {
        let fire = self.fuel > 0 && (self.recipe(world, slots).is_some() || self.progress > 0);
        vec![self.progress, self.fuel, i32::from(fire)]
    }
}

pub const SPEC: MachineSpec = MachineSpec {
    block: BLOCK,
    menu: "ntm:blast_furnace",
    slots: &[SlotKind::Input, SlotKind::Input, SlotKind::Fuel, SlotKind::Output],
    inputs: &[0, 1, 2],
    outputs: &[3],
    role: Role::None,
    max_power: 0,
    screen: &SCREEN,
};

/// `GUIDiFurnace`.
pub const SCREEN: Screen = Screen {
    texture: "ntm:gui/gui_difurnace",
    size: [176.0, 166.0],
    slots: &[Some([80.0, 18.0]), Some([80.0, 54.0]), Some([8.0, 36.0]), Some([134.0, 36.0])],
    inventory: [8.0, 84.0],
    title: Label { x: 88.0, y: 6.0, color: DARK },
    inventory_label: [8.0, 72.0],
    parts: &[
        Part::Bar { value: Value::Data(1), max: Value::Const(DiFurnace::MAX_FUEL as i64), fill: Fill::Up, extra: 0.0, src: [201.0, 1.0], dst: [44.0, 18.0], size: [16.0, 52.0] },
        Part::Bar { value: Value::Data(0), max: Value::Const(DiFurnace::PROCESSING_SPEED as i64), fill: Fill::Right, extra: 1.0, src: [176.0, 14.0], dst: [101.0, 35.0], size: [24.0, 17.0] },
        Part::When { value: Value::Data(2), min: 1, src: [176.0, 0.0], dst: [63.0, 37.0], size: [14.0, 14.0] },
    ],
    tips: &[],
    buttons: &[],
};

#[cfg(test)]
mod tests {
    use super::super::testing::{Stack, World, stack};
    use super::*;

    fn furnace(a: Option<Stack>, b: Option<Stack>, fuel: Option<Stack>) -> Vec<Option<Stack>> {
        vec![a, b, fuel, None]
    }

    #[test]
    fn fuel_goes_in_while_it_fits() {
        let (mut f, w) = (DiFurnace::default(), World::default());
        let mut slots = furnace(None, None, stack("coal", 2));
        f.tick(&w, &mut slots);
        assert_eq!((f.fuel, slots[2].clone()), (200, stack("coal", 1)));
        f.fuel = DiFurnace::MAX_FUEL - 199;
        f.tick(&w, &mut slots);
        assert_eq!(slots[2], stack("coal", 1), "no room for 200 more");
    }

    #[test]
    fn iron_and_coal_make_steel_in_400_ticks_of_fuel() {
        let (mut f, w) = (DiFurnace { fuel: 1_000, progress: 0 }, World::default());
        let mut slots = furnace(stack("iron", 3), stack("coal", 3), None);
        for _ in 0..399 {
            f.tick(&w, &mut slots);
        }
        assert_eq!((f.progress, slots[3].clone()), (399, None));
        f.tick(&w, &mut slots);
        assert_eq!((f.progress, f.fuel), (0, 600));
        assert_eq!(slots, furnace(stack("iron", 2), stack("coal", 2), None).into_iter().take(3).chain([stack("steel_ingot", 1)]).collect::<Vec<_>>());
    }

    #[test]
    fn running_dry_loses_the_progress() {
        let (mut f, w) = (DiFurnace { fuel: 200, progress: 0 }, World::default());
        let mut slots = furnace(stack("iron", 1), stack("coal", 1), None);
        for _ in 0..201 {
            f.tick(&w, &mut slots);
        }
        assert_eq!((f.fuel, f.progress, slots[3].clone()), (0, 0, None));
    }

    #[test]
    fn the_extension_triples_the_speed() {
        let mut w = World::default();
        w.neighbors.insert([0, 1, 0], EXTENSION);
        let mut f = DiFurnace { fuel: 1_000, progress: 0 };
        let mut slots = furnace(stack("iron", 1), stack("coal", 1), None);
        for _ in 0..133 {
            f.tick(&w, &mut slots);
        }
        assert_eq!(slots[3], None);
        f.tick(&w, &mut slots);
        assert_eq!((f.progress, slots[3].clone(), f.fuel), (2, stack("steel_ingot", 1), 866));
    }

    #[test]
    fn a_recipe_takes_the_counts_it_names() {
        let w = World::default();
        let mut f = DiFurnace { fuel: 1_000, progress: 399 };
        let mut slots = furnace(stack("copper", 1), stack("redstone", 1), None);
        f.tick(&w, &mut slots);
        assert_eq!((f.progress, slots[3].clone()), (0, None), "two redstone needed");
        slots[1] = stack("redstone", 2);
        f.progress = 399;
        f.tick(&w, &mut slots);
        assert_eq!((slots[0].clone(), slots[1].clone(), slots[3].clone()), (None, None, stack("red_copper", 2)));
    }

    #[test]
    fn lava_leaves_its_bucket() {
        let (mut f, w) = (DiFurnace::default(), World::default());
        let mut slots = furnace(None, None, stack("lava", 1));
        f.tick(&w, &mut slots);
        assert_eq!((f.fuel, slots[2].clone()), (DiFurnace::MAX_FUEL, stack("bucket", 1)));
    }
}
