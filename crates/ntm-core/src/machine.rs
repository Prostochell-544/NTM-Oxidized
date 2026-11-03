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
}
