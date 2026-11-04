//! `TileEntityMachineShredder` and `GUIMachineShredder`.

use super::screen::{DARK, Fill, Label, Part, Screen, Text, Tip, Value};
use super::{MachineSpec, MachineWorld, Role, SlotKind};
use crate::battery;

pub const BLOCK: &str = "ntm:machine_shredder";

/// `TileEntityMachineShredder`, with the Reinhardt port's limit: a tick needs its whole 5 HE.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shredder {
    pub power: i64,
    pub progress: i32,
}

impl Shredder {
    pub const MAX_POWER: i64 = 10_000;
    pub const PROCESSING_SPEED: i32 = 60;
    pub const POWER_PER_TICK: i64 = 5;
    pub const SLOTS: usize = 30;
    pub const INPUTS: std::ops::Range<usize> = 0..9;
    pub const OUTPUTS: std::ops::Range<usize> = 9..27;
    pub const LEFT_BLADE: usize = 27;
    pub const RIGHT_BLADE: usize = 28;
    pub const BATTERY: usize = 29;

    /// `getGearLeft`: 0 no blade, 1 good, 2 worn, 3 broken.
    pub fn gear<S>(world: &impl MachineWorld<S>, stack: Option<&S>) -> i32 {
        match stack.and_then(|stack| world.blade_damage(stack)) {
            None => 0,
            Some((_, 0)) => 1,
            Some((damage, max)) if damage < max / 2 => 1,
            Some((damage, max)) if damage < max => 2,
            Some(_) => 3,
        }
    }

    fn has_space<S>(world: &impl MachineWorld<S>, slots: &[Option<S>], stack: &S) -> bool {
        let Some(result) = world.shredder_result(stack) else {
            return false;
        };
        slots[Self::OUTPUTS].iter().any(|slot| super::fits(world, slot, &result))
    }

    fn can_process<S>(world: &impl MachineWorld<S>, slots: &[Option<S>]) -> bool {
        let usable = |slot: usize| matches!(Self::gear(world, slots[slot].as_ref()), 1 | 2);
        usable(Self::LEFT_BLADE)
            && usable(Self::RIGHT_BLADE)
            && slots[Self::INPUTS]
                .iter()
                .flatten()
                .any(|stack| world.count(stack) > 0 && Self::has_space(world, slots, stack))
    }

    fn damage_blade<S>(world: &impl MachineWorld<S>, slot: &mut Option<S>) {
        if let Some(stack) = slot
            && let Some((damage, max)) = world.blade_damage(stack)
            && max > 0
        {
            world.set_damage(stack, (damage + 1).min(max));
        }
    }

    /// `processItem`: one item from every input that has room, each into the first slot that
    /// takes it whole, a stack of the same item before an empty slot.
    fn process_items<S: Clone>(world: &impl MachineWorld<S>, slots: &mut [Option<S>]) {
        for input in Self::INPUTS {
            let Some(stack) = slots[input].clone() else {
                continue;
            };
            let Some(result) = world.shredder_result(&stack) else {
                continue;
            };
            let same = Self::OUTPUTS
                .into_iter()
                .find(|&out| slots[out].is_some() && super::fits(world, &slots[out], &result));
            let Some(out) = same.or_else(|| Self::OUTPUTS.into_iter().find(|&out| slots[out].is_none()))
            else {
                continue;
            };
            super::put(world, &mut slots[out], &result);
            super::take(world, &mut slots[input], 1);
        }
    }

    /// One tick: the work, then the battery slot tops the power up, as NTM orders them.
    pub fn tick<S: Clone>(&mut self, world: &impl MachineWorld<S>, slots: &mut [Option<S>]) {
        if self.power >= Self::POWER_PER_TICK && Self::can_process(world, slots) {
            self.power -= Self::POWER_PER_TICK;
            self.progress += 1;
            if self.progress >= Self::PROCESSING_SPEED {
                Self::damage_blade(world, &mut slots[Self::LEFT_BLADE]);
                Self::damage_blade(world, &mut slots[Self::RIGHT_BLADE]);
                Self::process_items(world, slots);
                self.progress = 0;
            }
        } else {
            self.progress = 0;
        }
        self.power = battery::discharge_slot(world, &mut slots[Self::BATTERY], self.power, Self::MAX_POWER);
    }

    /// The menu data [`SCREEN`] reads: power, progress, left and right gear.
    pub fn data<S>(&self, world: &impl MachineWorld<S>, slots: &[Option<S>]) -> Vec<i32> {
        let left = Self::gear(world, slots[Self::LEFT_BLADE].as_ref());
        let right = Self::gear(world, slots[Self::RIGHT_BLADE].as_ref());
        vec![self.power as i32, self.progress, left, right]
    }
}

use SlotKind::{Battery, Blade, Input, Output};

pub const SPEC: MachineSpec = MachineSpec {
    block: BLOCK,
    menu: "ntm:shredder",
    slots: &[
        Input, Input, Input, Input, Input, Input, Input, Input, Input,
        Output, Output, Output, Output, Output, Output, Output, Output, Output,
        Output, Output, Output, Output, Output, Output, Output, Output, Output,
        Blade, Blade, Battery,
    ],
    inputs: &[0, 1, 2, 3, 4, 5, 6, 7, 8],
    outputs: &[9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
    role: Role::Receiver,
    max_power: Shredder::MAX_POWER,
    screen: &SCREEN,
};

const fn grid(x: f32, slot: usize) -> Option<[f32; 2]> {
    Some([x + (slot % 3) as f32 * 18.0, 18.0 + (slot / 3) as f32 * 18.0])
}

/// `GUIMachineShredder`.
pub const SCREEN: Screen = Screen {
    texture: "ntm:gui/gui_shredder",
    size: [176.0, 233.0],
    slots: &[
        grid(44.0, 0), grid(44.0, 1), grid(44.0, 2), grid(44.0, 3), grid(44.0, 4), grid(44.0, 5),
        grid(44.0, 6), grid(44.0, 7), grid(44.0, 8),
        grid(116.0, 0), grid(116.0, 1), grid(116.0, 2), grid(116.0, 3), grid(116.0, 4), grid(116.0, 5),
        grid(116.0, 6), grid(116.0, 7), grid(116.0, 8), grid(116.0, 9), grid(116.0, 10), grid(116.0, 11),
        grid(116.0, 12), grid(116.0, 13), grid(116.0, 14), grid(116.0, 15), grid(116.0, 16), grid(116.0, 17),
        Some([44.0, 108.0]), Some([80.0, 108.0]), Some([8.0, 108.0]),
    ],
    inventory: [8.0, 151.0],
    // Centred over the panel right of the power bar, not over the whole screen.
    title: Label { x: 106.0, y: 6.0, color: DARK },
    inventory_label: [8.0, 139.0],
    parts: &[
        Part::Bar { value: Value::Data(0), max: Value::Const(Shredder::MAX_POWER), fill: Fill::Up, extra: 0.0, src: [176.0, 72.0], dst: [8.0, 18.0], size: [16.0, 88.0] },
        Part::Bar { value: Value::Data(1), max: Value::Const(Shredder::PROCESSING_SPEED as i64), fill: Fill::Right, extra: 1.0, src: [176.0, 54.0], dst: [63.0, 89.0], size: [34.0, 18.0] },
        Part::Pick { value: Value::Data(2), base: 1, step: [0.0, 18.0], hide: Some(0), src: [176.0, 0.0], dst: [43.0, 71.0], size: [18.0, 18.0] },
        Part::Pick { value: Value::Data(3), base: 1, step: [0.0, 18.0], hide: Some(0), src: [194.0, 0.0], dst: [79.0, 71.0], size: [18.0, 18.0] },
    ],
    tips: &[
        Tip { area: [8.0, 18.0, 16.0, 88.0], text: Text::Energy { value: Value::Data(0), max: Value::Const(Shredder::MAX_POWER) } },
        Tip {
            area: [43.0, 71.0, 54.0, 18.0],
            text: Text::Lines {
                lines: &["Error: Shredder blades are broken or missing!"],
                when: Some((&[Value::Data(2), Value::Data(3)], &[0, 3])),
            },
        },
    ],
    buttons: &[],
};

#[cfg(test)]
mod tests {
    use super::super::testing::{Stack, World, stack};
    use super::*;

    fn with_blades() -> Vec<Option<Stack>> {
        let mut slots = vec![None; Shredder::SLOTS];
        slots[Shredder::LEFT_BLADE] = stack("steel", 1);
        slots[Shredder::RIGHT_BLADE] = stack("steel", 1);
        slots
    }

    fn charged() -> Shredder {
        Shredder { power: Shredder::MAX_POWER, progress: 0 }
    }

    #[test]
    fn a_cycle_takes_60_ticks_and_300_he() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("ore", 3);
        for tick in 1..60 {
            s.tick(&w, &mut slots);
            assert_eq!(s.progress, tick);
        }
        assert_eq!(slots[9], None);
        s.tick(&w, &mut slots);
        assert_eq!(s.progress, 0);
        assert_eq!(s.power, Shredder::MAX_POWER - 300);
        assert_eq!(slots[0], stack("ore", 2));
        assert_eq!(slots[9], stack("powder", 2));
    }

    #[test]
    fn one_item_from_each_input_per_cycle() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("ore", 1);
        slots[4] = stack("ingot", 5);
        for _ in 0..60 {
            s.tick(&w, &mut slots);
        }
        assert_eq!(slots[0], None);
        assert_eq!(slots[4], stack("ingot", 4));
        assert_eq!(slots[9], stack("powder", 3));
    }

    #[test]
    fn what_has_no_recipe_becomes_scrap() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("dirt", 1);
        for _ in 0..60 {
            s.tick(&w, &mut slots);
        }
        assert_eq!(slots[9], stack("scrap", 1));
    }

    #[test]
    fn it_needs_two_blades_and_blades_that_arent_broken() {
        let w = World::default();
        for (left, right, runs) in [
            (None, stack("steel", 1), false),
            (stack("steel", 1), None, false),
            (stack("desh", 1), stack("steel", 1), true),
        ] {
            let (mut s, mut slots) = (charged(), with_blades());
            slots[0] = stack("ore", 1);
            slots[Shredder::LEFT_BLADE] = left;
            slots[Shredder::RIGHT_BLADE] = right;
            s.tick(&w, &mut slots);
            assert_eq!(s.progress, i32::from(runs));
        }
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[Shredder::LEFT_BLADE].as_mut().unwrap().damage = 200;
        s.tick(&w, &mut slots);
        assert_eq!(s.progress, 0);
    }

    #[test]
    fn blades_wear_one_per_cycle_and_show_it() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("ore", 1);
        slots[Shredder::RIGHT_BLADE] = stack("desh", 1);
        for _ in 0..60 {
            s.tick(&w, &mut slots);
        }
        assert_eq!(slots[Shredder::LEFT_BLADE].as_ref().unwrap().damage, 1);
        assert_eq!(slots[Shredder::RIGHT_BLADE].as_ref().unwrap().damage, 0);

        let worn = |damage| {
            let mut blade = stack("steel", 1).unwrap();
            blade.damage = damage;
            Shredder::gear(&w, Some(&blade))
        };
        assert_eq!(Shredder::gear(&w, None::<&Stack>), 0);
        assert_eq!((worn(99), worn(100), worn(199), worn(200)), (1, 2, 2, 3));
    }

    #[test]
    fn without_5_he_or_room_progress_goes_back_to_zero() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("ore", 1);
        s.tick(&w, &mut slots);
        s.power = 4;
        s.tick(&w, &mut slots);
        assert_eq!((s.progress, s.power), (0, 4));

        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        for out in Shredder::OUTPUTS {
            slots[out] = stack("dirt", 64);
        }
        s.tick(&w, &mut slots);
        assert_eq!((s.progress, s.power), (0, Shredder::MAX_POWER));
    }

    #[test]
    fn a_full_stack_goes_on_to_the_next_slot() {
        let (mut s, mut slots, w) = (charged(), with_blades(), World::default());
        slots[0] = stack("ore", 1);
        slots[9] = stack("powder", 63);
        for _ in 0..60 {
            s.tick(&w, &mut slots);
        }
        assert_eq!(slots[9], stack("powder", 63));
        assert_eq!(slots[10], stack("powder", 2));
    }

    #[test]
    fn the_battery_slot_tops_the_power_up_after_the_work() {
        let battery = |charge| Some(Stack { item: "battery", count: 1, damage: charge });
        let (mut s, mut slots, w) = (Shredder::default(), with_blades(), World::default());
        slots[Shredder::BATTERY] = battery(5_000);
        s.tick(&w, &mut slots);
        assert_eq!((s.power, slots[Shredder::BATTERY].clone()), (1_000, battery(4_000)));

        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[Shredder::BATTERY] = battery(5_000);
        s.tick(&w, &mut slots);
        assert_eq!((s.progress, s.power), (1, Shredder::MAX_POWER));
        assert_eq!(slots[Shredder::BATTERY], battery(4_995));
    }

    #[test]
    fn the_screen_shows_power_progress_and_broken_blades() {
        let blits = super::super::screen::blits(&SCREEN, &[5_000, 30, 1, 3]);
        assert_eq!(blits[1].dst, [8.0, 62.0, 16.0, 44.0]);
        assert_eq!(blits[2].dst[2], 18.0);
        assert_eq!(blits[4].src, [194.0, 36.0, 18.0, 18.0]);
        let tip = super::super::screen::tooltip(&SCREEN, &[5_000, 30, 1, 3], [50.0, 75.0]);
        assert_eq!(tip, vec!["Error: Shredder blades are broken or missing!"]);
        assert!(super::super::screen::tooltip(&SCREEN, &[5_000, 30, 1, 1], [50.0, 75.0]).is_empty());
    }
}
