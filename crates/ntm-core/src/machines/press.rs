//! `TileEntityMachinePress`, the burner press, and `GUIMachinePress`. One block for now: the
//! original is three tall. Its speed gauge's needle isn't drawn; its tooltip tells the speed.

use super::screen::{Fill, Label, Part, Screen, Text, Tip, Value};
use super::{MachineSpec, MachineWorld, Nbt, Role, SlotKind, STAMP_PREFIX};

pub const BLOCK: &str = "ntm:machine_press";
/// Next to the press it heats it: the speed builds four times as fast.
pub const PREHEATER: &str = "ntm:press_preheater";

/// The stamp type an item is (`ItemStamp.StampType`), as press recipes name it.
pub fn stamp_type(id: &str) -> Option<&'static str> {
    let name = id.strip_prefix(STAMP_PREFIX)?;
    Some(match name.rsplit('_').next()? {
        "flat" => "flat",
        "plate" => "plate",
        "wire" => "wire",
        "circuit" => "circuit",
        "357" => "c357",
        "44" => "c44",
        "9" => "c9",
        "50" => "c50",
        _ => return None,
    })
}

/// How many operations a stamp lasts (`ItemStamp`'s durability in CE's `ModItems`); 0 never wears.
pub fn stamp_durability(id: &str) -> u32 {
    let Some(name) = id.strip_prefix(STAMP_PREFIX) else {
        return 0;
    };
    match name.split('_').next() {
        Some("stone") => 32,
        Some("iron") => 64,
        Some("steel") => 192,
        Some("titanium") => 256,
        Some("obsidian") => 512,
        Some("357" | "44" | "9" | "50") => 1000,
        _ => 0,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Press {
    /// Builds up while it works, falls while it doesn't.
    pub speed: i32,
    /// Burn ticks of the fuel taken; one operation costs 200.
    pub burn_time: i32,
    /// How far the stamp is down; an operation is done at `MAX_PRESS`.
    pub press: i32,
    pub retracting: bool,
    /// Ticks to wait before the stamp turns round.
    pub delay: i32,
}

impl Press {
    pub const MAX_SPEED: i32 = 400;
    pub const PROGRESS_AT_MAX: i32 = 25;
    pub const MAX_PRESS: i32 = 200;
    pub const OPERATION: i32 = 200;
    pub const FUEL: usize = 0;
    pub const STAMP: usize = 1;
    pub const INPUT: usize = 2;
    pub const OUTPUT: usize = 3;
    pub const SLOTS: usize = 13;

    pub fn load(nbt: &impl Nbt) -> Press {
        Press {
            speed: nbt.int("speed"),
            burn_time: nbt.int("burnTime"),
            press: nbt.int("press"),
            retracting: nbt.int("ret") != 0,
            delay: nbt.int("delay"),
        }
    }

    pub fn save(&self, nbt: &mut impl Nbt) {
        nbt.set_int("speed", self.speed);
        nbt.set_int("burnTime", self.burn_time);
        nbt.set_int("press", self.press);
        nbt.set_int("ret", i32::from(self.retracting));
        nbt.set_int("delay", self.delay);
    }

    fn result<S>(&self, world: &impl MachineWorld<S>, slots: &[Option<S>]) -> Option<S> {
        if self.burn_time < Self::OPERATION {
            return None;
        }
        let (Some(stamp), Some(input)) = (&slots[Self::STAMP], &slots[Self::INPUT]) else {
            return None;
        };
        let out = world.press_result(world.stamp(stamp)?, input)?;
        super::fits(world, &slots[Self::OUTPUT], &out).then_some(out)
    }

    pub fn tick<S: Clone>(&mut self, world: &impl MachineWorld<S>, slots: &mut [Option<S>]) {
        const SIDES: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
        let preheated = SIDES.iter().any(|&side| world.neighbor(side) == PREHEATER);
        let result = self.result(world, slots);

        if (result.is_some() || self.retracting) && self.burn_time >= Self::OPERATION {
            self.speed = (self.speed + if preheated { 4 } else { 1 }).min(Self::MAX_SPEED);
        } else {
            self.speed = (self.speed - 1).max(0);
        }

        if self.delay <= 0 {
            let stamp_speed = self.speed * Self::PROGRESS_AT_MAX / Self::MAX_SPEED;
            if self.retracting {
                self.press -= stamp_speed;
                if self.press <= 0 {
                    self.retracting = false;
                    self.delay = 5;
                }
            } else if let Some(out) = result {
                self.press += stamp_speed;
                if self.press >= Self::MAX_PRESS {
                    super::put(world, &mut slots[Self::OUTPUT], &out);
                    super::take(world, &mut slots[Self::INPUT], 1);
                    if let Some(stamp) = &mut slots[Self::STAMP] {
                        let (damage, max) = world.damage(stamp);
                        if max != 0 {
                            world.set_damage(stamp, damage + 1);
                            if damage + 1 >= max {
                                slots[Self::STAMP] = None;
                            }
                        }
                    }
                    self.retracting = true;
                    self.delay = 5;
                    if self.burn_time >= Self::OPERATION {
                        self.burn_time -= Self::OPERATION;
                    }
                }
            } else if self.press > 0 {
                self.retracting = true;
            }
        } else {
            self.delay -= 1;
        }

        // Less than an operation's worth left: burn the next fuel.
        if let Some(fuel) = &slots[Self::FUEL] {
            let burn = world.burn_time(fuel);
            if self.burn_time < Self::OPERATION && burn > 0 {
                self.burn_time += burn;
                super::burn_one(world, &mut slots[Self::FUEL]);
            }
        }
    }

    /// The menu data [`SCREEN`] reads: speed, burn time, how far the stamp is down.
    pub fn data(&self) -> Vec<i32> {
        vec![self.speed, self.burn_time, self.press]
    }
}

use SlotKind::{Fuel, Input, Output, Stamp, Storage};

pub const SPEC: MachineSpec = MachineSpec {
    block: BLOCK,
    menu: "ntm:press",
    slots: &[Fuel, Stamp, Input, Output, Storage, Storage, Storage, Storage, Storage, Storage, Storage, Storage, Storage],
    inputs: &[0, 1, 2],
    outputs: &[3],
    role: Role::None,
    max_power: 0,
    screen: &SCREEN,
};

const fn row(i: usize) -> Option<[f32; 2]> {
    Some([8.0 + i as f32 * 18.0, 84.0])
}

/// `GUIMachinePress`.
pub const SCREEN: Screen = Screen {
    texture: "ntm:gui/gui_press",
    size: [176.0, 214.0],
    slots: &[
        Some([26.0, 53.0]), Some([80.0, 17.0]), Some([80.0, 53.0]), Some([140.0, 35.0]),
        row(0), row(1), row(2), row(3), row(4), row(5), row(6), row(7), row(8),
    ],
    inventory: [8.0, 132.0],
    title: Label { x: 88.0, y: 5.0, color: 0xFFFFFF },
    inventory_label: [8.0, 120.0],
    parts: &[
        Part::When { value: Value::Data(1), min: 20, src: [0.0, 214.0], dst: [26.0, 36.0], size: [14.0, 14.0] },
        Part::Bar { value: Value::Data(2), max: Value::Const(Press::MAX_PRESS as i64), fill: Fill::Down, extra: 0.0, src: [15.0, 214.0], dst: [79.0, 35.0], size: [18.0, 16.0] },
    ],
    tips: &[
        Tip { area: [25.0, 16.0, 18.0, 18.0], text: Text::Scaled { value: Value::Data(0), num: 100, den: Press::MAX_SPEED as i64, suffix: "%" } },
        Tip { area: [25.0, 34.0, 18.0, 18.0], text: Text::Scaled { value: Value::Data(1), num: 1, den: Press::OPERATION as i64, suffix: " operations left" } },
    ],
    buttons: &[],
};

#[cfg(test)]
mod tests {
    use super::super::testing::{Stack, World, stack};
    use super::*;

    fn loaded() -> Vec<Option<Stack>> {
        let mut slots = vec![None; Press::SLOTS];
        slots[Press::STAMP] = stack("stamp_plate", 1);
        slots[Press::INPUT] = stack("ingot", 2);
        slots
    }

    #[test]
    fn stamps_are_named_by_their_last_word() {
        assert_eq!(stamp_type("ntm:stamp_iron_plate"), Some("plate"));
        assert_eq!(stamp_type("ntm:stamp_desh_357"), Some("c357"));
        assert_eq!(stamp_type("ntm:stamp_9"), Some("c9"));
        assert_eq!(stamp_type("ntm:ingot_iron"), None);
        assert_eq!((stamp_durability("ntm:stamp_steel_wire"), stamp_durability("ntm:stamp_desh_flat")), (192, 0));
        assert_eq!(stamp_durability("ntm:stamp_50"), 1000);
    }

    #[test]
    fn from_cold_the_first_plate_comes_on_tick_87() {
        let w = World::default();
        let mut p = Press { burn_time: 1_600, ..Press::default() };
        let mut slots = loaded();
        for _ in 0..86 {
            p.tick(&w, &mut slots);
        }
        assert_eq!((p.press, slots[Press::OUTPUT].clone()), (195, None));
        p.tick(&w, &mut slots);
        assert_eq!(slots[Press::OUTPUT], stack("plate", 1));
        assert_eq!((p.burn_time, p.retracting, p.delay), (1_400, true, 5));
        assert_eq!((slots[Press::INPUT].clone(), slots[Press::STAMP].as_ref().unwrap().damage), (stack("ingot", 1), 1));
    }

    #[test]
    fn a_worn_out_stamp_breaks() {
        let w = World::default();
        let mut p = Press { burn_time: 200, speed: 400, press: 199, ..Press::default() };
        let mut slots = loaded();
        slots[Press::STAMP].as_mut().unwrap().damage = 2;
        p.tick(&w, &mut slots);
        assert_eq!((slots[Press::STAMP].clone(), slots[Press::OUTPUT].clone()), (None, stack("plate", 1)));
        assert_eq!(p.burn_time, 0);
    }

    #[test]
    fn it_burns_fuel_below_one_operation_and_slows_down_without() {
        let w = World::default();
        let mut p = Press { speed: 10, ..Press::default() };
        let mut slots = loaded();
        slots[Press::FUEL] = stack("coal", 2);
        p.tick(&w, &mut slots);
        assert_eq!((p.speed, p.burn_time, slots[Press::FUEL].clone()), (9, 1_600, stack("coal", 1)));
        p.tick(&w, &mut slots);
        assert_eq!((p.speed, slots[Press::FUEL].clone()), (10, stack("coal", 1)), "enough fuel stored: none taken");
    }

    #[test]
    fn a_preheater_builds_speed_four_times_as_fast() {
        let mut w = World::default();
        w.neighbors.insert([1, 0, 0], PREHEATER);
        let mut p = Press { burn_time: 1_600, ..Press::default() };
        let mut slots = loaded();
        p.tick(&w, &mut slots);
        assert_eq!(p.speed, 4);
    }
}
