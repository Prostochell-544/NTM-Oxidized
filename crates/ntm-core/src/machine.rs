//! Machine logic, without the world around it.

/// What a fuel is to `ModuleBurnTime`, which times some fuels differently per machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuelKind {
    Log,
    Wood,
    Other,
}

/// `TileEntityMachineWoodBurner` burning solid fuel. Its ash and its liquid mode aren't here yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WoodBurner {
    pub power: i64,
    pub burn_time: i32,
    pub max_burn_time: i32,
    pub is_on: bool,
}

impl WoodBurner {
    pub const MAX_POWER: i64 = 100_000;
    const POWER_PER_TICK: i64 = 100;

    /// How long a fuel burning for `base` ticks in a furnace burns here.
    pub fn burn_time_of(base: i32, kind: FuelKind) -> i32 {
        let modifier = match kind {
            FuelKind::Log => 4.0,
            FuelKind::Wood => 2.0,
            FuelKind::Other => 1.0,
        };
        (f64::from(base) * modifier) as i32
    }

    /// One tick, with `fuel` the burn time of what is in the fuel slot, 0 for nothing that burns.
    /// Answers whether the fuel was used.
    pub fn tick(&mut self, fuel: i32) -> bool {
        let mut generated = 0;
        let mut used_fuel = false;
        if self.burn_time <= 0 {
            if fuel > 0 {
                self.burn_time = fuel;
                self.max_burn_time = fuel;
                used_fuel = true;
            }
        } else if self.power < Self::MAX_POWER && self.is_on {
            self.burn_time -= 1;
            generated = Self::POWER_PER_TICK;
        }
        self.power = (self.power + generated).min(Self::MAX_POWER);
        used_fuel
    }
}

/// How the shredder sees the stacks of its slots, whatever they are.
pub trait ShredderRules<S> {
    fn same_item(&self, a: &S, b: &S) -> bool;
    fn count(&self, stack: &S) -> u32;
    fn set_count(&self, stack: &mut S, count: u32);
    fn max_stack(&self, stack: &S) -> u32;
    /// A blade's `(damage, max damage)`, none for anything else. Max damage 0 never wears out.
    fn blade_damage(&self, stack: &S) -> Option<(u32, u32)>;
    fn set_damage(&self, stack: &mut S, damage: u32);
    /// What `stack` shreds into. NTM makes scrap of what has no recipe.
    fn result(&self, stack: &S) -> Option<S>;
}

/// `TileEntityMachineShredder`, with the Reinhardt port's limit: a tick needs its whole 5 HE. The
/// battery slot does nothing yet.
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

    /// `getGearLeft`: 0 no blade, 1 good, 2 worn, 3 broken.
    pub fn gear<S>(rules: &impl ShredderRules<S>, stack: Option<&S>) -> i32 {
        match stack.and_then(|stack| rules.blade_damage(stack)) {
            None => 0,
            Some((_, 0)) => 1,
            Some((damage, max)) if damage < max / 2 => 1,
            Some((damage, max)) if damage < max => 2,
            Some(_) => 3,
        }
    }

    fn has_space<S>(rules: &impl ShredderRules<S>, slots: &[Option<S>], stack: &S) -> bool {
        let Some(result) = rules.result(stack) else {
            return false;
        };
        slots[Self::OUTPUTS].iter().any(|slot| match slot {
            None => true,
            Some(out) => {
                rules.same_item(out, &result)
                    && rules.count(out) + rules.count(&result) <= rules.max_stack(&result)
            }
        })
    }

    fn can_process<S>(rules: &impl ShredderRules<S>, slots: &[Option<S>]) -> bool {
        let usable = |slot: usize| matches!(Self::gear(rules, slots[slot].as_ref()), 1 | 2);
        usable(Self::LEFT_BLADE)
            && usable(Self::RIGHT_BLADE)
            && slots[Self::INPUTS]
                .iter()
                .flatten()
                .any(|stack| rules.count(stack) > 0 && Self::has_space(rules, slots, stack))
    }

    fn damage_blade<S>(rules: &impl ShredderRules<S>, slot: &mut Option<S>) {
        if let Some(stack) = slot
            && let Some((damage, max)) = rules.blade_damage(stack)
            && max > 0
        {
            rules.set_damage(stack, (damage + 1).min(max));
        }
    }

    /// `processItem`: one item from every input that has room, each into the first slot that
    /// takes it whole, a stack of the same item before an empty slot.
    fn process_items<S: Clone>(rules: &impl ShredderRules<S>, slots: &mut [Option<S>]) {
        for input in Self::INPUTS {
            let Some(stack) = slots[input].clone() else {
                continue;
            };
            let Some(result) = rules.result(&stack) else {
                continue;
            };
            let added = rules.count(&result);
            let same = Self::OUTPUTS.into_iter().find(|&out| {
                slots[out].as_ref().is_some_and(|out| {
                    rules.same_item(out, &result)
                        && rules.count(out) + added <= rules.max_stack(&result)
                })
            });
            if let Some(out) = same {
                let out = slots[out].as_mut().expect("just found");
                let count = rules.count(out) + added;
                rules.set_count(out, count);
            } else if let Some(out) = Self::OUTPUTS.into_iter().find(|&out| slots[out].is_none()) {
                slots[out] = Some(result);
            } else {
                continue;
            }
            let left = rules.count(&stack) - 1;
            if left == 0 {
                slots[input] = None;
            } else if let Some(stack) = slots[input].as_mut() {
                rules.set_count(stack, left);
            }
        }
    }

    pub fn tick<S: Clone>(&mut self, rules: &impl ShredderRules<S>, slots: &mut [Option<S>]) {
        if self.power < Self::POWER_PER_TICK || !Self::can_process(rules, slots) {
            self.progress = 0;
            return;
        }
        self.power -= Self::POWER_PER_TICK;
        self.progress += 1;
        if self.progress >= Self::PROCESSING_SPEED {
            Self::damage_blade(rules, &mut slots[Self::LEFT_BLADE]);
            Self::damage_blade(rules, &mut slots[Self::RIGHT_BLADE]);
            Self::process_items(rules, slots);
            self.progress = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burner() -> WoodBurner {
        WoodBurner {
            is_on: true,
            ..WoodBurner::default()
        }
    }

    #[test]
    fn logs_burn_four_times_as_long_and_wood_twice() {
        assert_eq!(WoodBurner::burn_time_of(300, FuelKind::Log), 1200);
        assert_eq!(WoodBurner::burn_time_of(300, FuelKind::Wood), 600);
        assert_eq!(WoodBurner::burn_time_of(1600, FuelKind::Other), 1600);
    }

    #[test]
    fn taking_fuel_is_a_tick_of_its_own() {
        let mut b = burner();
        assert!(b.tick(300));
        assert_eq!((b.power, b.burn_time, b.max_burn_time), (0, 300, 300));
        assert!(!b.tick(300));
        assert_eq!((b.power, b.burn_time), (100, 299));
    }

    #[test]
    fn a_fuel_gives_100_he_per_tick_it_burns() {
        let mut b = burner();
        assert!(b.tick(3));
        for _ in 0..3 {
            assert!(!b.tick(0));
        }
        assert_eq!((b.power, b.burn_time), (300, 0));
        assert!(!b.tick(0));
        assert_eq!(b.power, 300);
    }

    #[test]
    fn it_stops_burning_when_full_and_stays_at_the_limit() {
        let mut b = WoodBurner {
            power: 99_950,
            burn_time: 10,
            ..burner()
        };
        b.tick(0);
        assert_eq!((b.power, b.burn_time), (100_000, 9));
        b.tick(0);
        assert_eq!((b.power, b.burn_time), (100_000, 9));
    }

    #[test]
    fn switched_off_it_keeps_its_fire_but_still_takes_fuel_when_out() {
        let mut b = WoodBurner {
            is_on: false,
            burn_time: 10,
            ..WoodBurner::default()
        };
        b.tick(0);
        assert_eq!((b.power, b.burn_time), (0, 10));
        b.burn_time = 0;
        assert!(b.tick(300));
        assert_eq!(b.burn_time, 300);
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Stack {
        item: &'static str,
        count: u32,
        damage: u32,
    }

    fn stack(item: &'static str, count: u32) -> Option<Stack> {
        Some(Stack {
            item,
            count,
            damage: 0,
        })
    }

    struct Rules;

    impl ShredderRules<Stack> for Rules {
        fn same_item(&self, a: &Stack, b: &Stack) -> bool {
            a.item == b.item
        }
        fn count(&self, stack: &Stack) -> u32 {
            stack.count
        }
        fn set_count(&self, stack: &mut Stack, count: u32) {
            stack.count = count;
        }
        fn max_stack(&self, _: &Stack) -> u32 {
            64
        }
        fn blade_damage(&self, stack: &Stack) -> Option<(u32, u32)> {
            match stack.item {
                "steel" => Some((stack.damage, 200)),
                "desh" => Some((stack.damage, 0)),
                _ => None,
            }
        }
        fn set_damage(&self, stack: &mut Stack, damage: u32) {
            stack.damage = damage;
        }
        fn result(&self, input: &Stack) -> Option<Stack> {
            match input.item {
                "ore" => stack("powder", 2),
                "ingot" => stack("powder", 1),
                _ => stack("scrap", 1),
            }
        }
    }

    fn with_blades() -> Vec<Option<Stack>> {
        let mut slots = vec![None; Shredder::SLOTS];
        slots[Shredder::LEFT_BLADE] = stack("steel", 1);
        slots[Shredder::RIGHT_BLADE] = stack("steel", 1);
        slots
    }

    fn charged() -> Shredder {
        Shredder {
            power: Shredder::MAX_POWER,
            progress: 0,
        }
    }

    #[test]
    fn a_cycle_takes_60_ticks_and_300_he() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 3);
        for tick in 1..60 {
            s.tick(&Rules, &mut slots);
            assert_eq!(s.progress, tick);
        }
        assert_eq!(slots[9], None);
        s.tick(&Rules, &mut slots);
        assert_eq!(s.progress, 0);
        assert_eq!(s.power, Shredder::MAX_POWER - 300);
        assert_eq!(slots[0], stack("ore", 2));
        assert_eq!(slots[9], stack("powder", 2));
    }

    #[test]
    fn one_item_from_each_input_per_cycle() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[4] = stack("ingot", 5);
        for _ in 0..60 {
            s.tick(&Rules, &mut slots);
        }
        assert_eq!(slots[0], None);
        assert_eq!(slots[4], stack("ingot", 4));
        assert_eq!(slots[9], stack("powder", 3));
    }

    #[test]
    fn what_has_no_recipe_becomes_scrap() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("dirt", 1);
        for _ in 0..60 {
            s.tick(&Rules, &mut slots);
        }
        assert_eq!(slots[9], stack("scrap", 1));
    }

    #[test]
    fn it_needs_two_blades_and_blades_that_arent_broken() {
        for (left, right, runs) in [
            (None, stack("steel", 1), false),
            (stack("steel", 1), None, false),
            (stack("desh", 1), stack("steel", 1), true),
        ] {
            let (mut s, mut slots) = (charged(), with_blades());
            slots[0] = stack("ore", 1);
            slots[Shredder::LEFT_BLADE] = left;
            slots[Shredder::RIGHT_BLADE] = right;
            s.tick(&Rules, &mut slots);
            assert_eq!(s.progress, i32::from(runs));
        }
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[Shredder::LEFT_BLADE].as_mut().unwrap().damage = 200;
        s.tick(&Rules, &mut slots);
        assert_eq!(s.progress, 0);
    }

    #[test]
    fn blades_wear_one_per_cycle_and_show_it() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[Shredder::RIGHT_BLADE] = stack("desh", 1);
        for _ in 0..60 {
            s.tick(&Rules, &mut slots);
        }
        assert_eq!(slots[Shredder::LEFT_BLADE].as_ref().unwrap().damage, 1);
        assert_eq!(slots[Shredder::RIGHT_BLADE].as_ref().unwrap().damage, 0);

        let worn = |damage| {
            let mut blade = stack("steel", 1).unwrap();
            blade.damage = damage;
            Shredder::gear(&Rules, Some(&blade))
        };
        assert_eq!(Shredder::gear(&Rules, None), 0);
        assert_eq!((worn(99), worn(100), worn(199), worn(200)), (1, 2, 2, 3));
    }

    #[test]
    fn without_5_he_or_room_progress_goes_back_to_zero() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        s.tick(&Rules, &mut slots);
        s.power = 4;
        s.tick(&Rules, &mut slots);
        assert_eq!((s.progress, s.power), (0, 4));

        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        for out in Shredder::OUTPUTS {
            slots[out] = stack("dirt", 64);
        }
        s.tick(&Rules, &mut slots);
        assert_eq!((s.progress, s.power), (0, Shredder::MAX_POWER));
    }

    #[test]
    fn a_full_stack_goes_on_to_the_next_slot() {
        let (mut s, mut slots) = (charged(), with_blades());
        slots[0] = stack("ore", 1);
        slots[9] = stack("powder", 63);
        for _ in 0..60 {
            s.tick(&Rules, &mut slots);
        }
        assert_eq!(slots[9], stack("powder", 63));
        assert_eq!(slots[10], stack("powder", 2));
    }
}
