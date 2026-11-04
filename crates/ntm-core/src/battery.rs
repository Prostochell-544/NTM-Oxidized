//! Batteries: `IBatteryItem` and `ItemBatteryPack`, the battery slot helpers of `Library`, and
//! the settings of the storage blocks (`TileEntityBatteryBase`, `TileEntityBatterySocket`).

use crate::energy::Priority;
use crate::machines::screen::{Button, DARK, Fill, Label, Part, Screen, Text, Tip, Value};

pub const SOCKET_BLOCK: &str = "ntm:machine_battery_socket";
pub const SOCKET_MENU: &str = "ntm:battery_socket";

/// `EnumBatteryPack`, in its metadata order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryPack {
    BatteryRedstone,
    BatteryLead,
    BatteryLithium,
    BatterySodium,
    BatterySchrabidium,
    BatteryQuantum,
    CapacitorCopper,
    CapacitorGold,
    CapacitorNiobium,
    CapacitorTantalum,
    CapacitorBismuth,
    CapacitorSpark,
}

impl BatteryPack {
    pub const ALL: [BatteryPack; 12] = [
        BatteryPack::BatteryRedstone,
        BatteryPack::BatteryLead,
        BatteryPack::BatteryLithium,
        BatteryPack::BatterySodium,
        BatteryPack::BatterySchrabidium,
        BatteryPack::BatteryQuantum,
        BatteryPack::CapacitorCopper,
        BatteryPack::CapacitorGold,
        BatteryPack::CapacitorNiobium,
        BatteryPack::CapacitorTantalum,
        BatteryPack::CapacitorBismuth,
        BatteryPack::CapacitorSpark,
    ];

    /// Its name, the item `battery_pack_<name>` as Reinhardt flattened the metadata.
    pub fn name(self) -> &'static str {
        match self {
            BatteryPack::BatteryRedstone => "battery_redstone",
            BatteryPack::BatteryLead => "battery_lead",
            BatteryPack::BatteryLithium => "battery_lithium",
            BatteryPack::BatterySodium => "battery_sodium",
            BatteryPack::BatterySchrabidium => "battery_schrabidium",
            BatteryPack::BatteryQuantum => "battery_quantum",
            BatteryPack::CapacitorCopper => "capacitor_copper",
            BatteryPack::CapacitorGold => "capacitor_gold",
            BatteryPack::CapacitorNiobium => "capacitor_niobium",
            BatteryPack::CapacitorTantalum => "capacitor_tantalum",
            BatteryPack::CapacitorBismuth => "capacitor_bismuth",
            BatteryPack::CapacitorSpark => "capacitor_spark",
        }
    }

    pub fn from_name(name: &str) -> Option<BatteryPack> {
        BatteryPack::ALL.into_iter().find(|pack| pack.name() == name)
    }

    pub fn is_capacitor(self) -> bool {
        self as u8 > BatteryPack::BatteryQuantum as u8
    }

    /// The constructor's `dischargeRate`, in HE per tick, which sets the rest.
    fn rate(self) -> i64 {
        match self {
            BatteryPack::BatteryRedstone => 100,
            BatteryPack::BatteryLead => 1_000,
            BatteryPack::BatteryLithium => 10_000,
            BatteryPack::BatterySodium => 50_000,
            BatteryPack::BatterySchrabidium => 250_000,
            BatteryPack::BatteryQuantum => 1_000_000,
            BatteryPack::CapacitorCopper => 1_000,
            BatteryPack::CapacitorGold => 10_000,
            BatteryPack::CapacitorNiobium => 100_000,
            BatteryPack::CapacitorTantalum => 500_000,
            BatteryPack::CapacitorBismuth => 2_500_000,
            BatteryPack::CapacitorSpark => 10_000_000,
        }
    }

    /// A capacitor empties in 30 seconds, a battery in 15 minutes, the quantum battery in an hour.
    pub fn capacity(self) -> i64 {
        let rate = self.rate();
        match self {
            BatteryPack::BatteryQuantum => rate * 20 * 60 * 60,
            _ if self.is_capacitor() => rate * 20 * 30,
            _ => rate * 20 * 60 * 15,
        }
    }

    /// A capacitor charges as fast as it discharges, a battery ten times faster.
    pub fn charge_rate(self) -> i64 {
        if self.is_capacitor() { self.rate() } else { self.rate() * 10 }
    }

    pub fn discharge_rate(self) -> i64 {
        self.rate()
    }

    pub fn battery(self, charge: i64) -> Battery {
        Battery {
            charge,
            max_charge: self.capacity(),
            charge_rate: self.charge_rate(),
            discharge_rate: self.discharge_rate(),
        }
    }
}

/// A battery item as `IBatteryItem` answers for it: its charge, in HE, and its limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub charge: i64,
    pub max_charge: i64,
    pub charge_rate: i64,
    pub discharge_rate: i64,
}

impl Battery {
    /// `Library.chargeItemsFromTE`: a machine holding `power` of its `max_power` charges the battery.
    /// Answers the machine's power after. Java's quirks stay: power past the maximum comes back as
    /// the maximum, and negative power as none.
    pub fn charge_from(&mut self, power: i64, max_power: i64) -> i64 {
        if power < 0 {
            return 0;
        }
        if power > max_power {
            return max_power;
        }
        let to_charge = power
            .min(self.charge_rate)
            .min(self.max_charge.wrapping_sub(self.charge));
        self.charge = self.charge.wrapping_add(to_charge);
        power.wrapping_sub(to_charge)
    }

    /// `Library.chargeTEFromItems`: the battery feeds a machine holding `power` of its `max_power`.
    /// Answers the machine's power after.
    pub fn discharge_into(&mut self, power: i64, max_power: i64) -> i64 {
        let to_discharge = max_power
            .wrapping_sub(power)
            .min(self.discharge_rate)
            .min(self.charge);
        self.charge = self.charge.wrapping_sub(to_discharge);
        power.wrapping_add(to_discharge)
    }
}

/// How a machine sees what lies in its battery slot, whatever its stacks are.
pub trait BatterySlot<S> {
    /// The battery `stack` is, none for anything else.
    fn battery(&self, stack: &S) -> Option<Battery>;
    fn set_charge(&self, stack: &mut S, charge: i64);
}

/// [`Battery::discharge_into`] for the battery in `slot`, if there is one.
pub fn discharge_slot<S>(
    rules: &impl BatterySlot<S>,
    slot: &mut Option<S>,
    power: i64,
    max_power: i64,
) -> i64 {
    let Some(stack) = slot else {
        return power;
    };
    let Some(mut battery) = rules.battery(stack) else {
        return power;
    };
    let power = battery.discharge_into(power, max_power);
    rules.set_charge(stack, battery.charge);
    power
}

/// [`Battery::charge_from`] for the battery in `slot`, if there is one; the checks on `power` come
/// first, as in NTM.
pub fn charge_slot<S>(
    rules: &impl BatterySlot<S>,
    slot: &mut Option<S>,
    power: i64,
    max_power: i64,
) -> i64 {
    if power < 0 {
        return 0;
    }
    if power > max_power {
        return max_power;
    }
    let Some(stack) = slot else {
        return power;
    };
    let Some(mut battery) = rules.battery(stack) else {
        return power;
    };
    let power = battery.charge_from(power, max_power);
    rules.set_charge(stack, battery.charge);
    power
}

/// What a storage block is on its network: `mode_input`, `mode_buffer`, `mode_output`, `mode_none`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageMode {
    Input,
    Buffer,
    Output,
    None,
}

impl StorageMode {
    /// The saved number; anything else reads as none, which a switch in Java would skip.
    pub fn from_index(index: i32) -> StorageMode {
        match index {
            0 => StorageMode::Input,
            1 => StorageMode::Buffer,
            2 => StorageMode::Output,
            _ => StorageMode::None,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }

    fn next(self) -> StorageMode {
        StorageMode::from_index((self.index() + 1) % 4)
    }

    pub fn provides(self) -> bool {
        matches!(self, StorageMode::Output | StorageMode::Buffer)
    }

    pub fn receives(self) -> bool {
        matches!(self, StorageMode::Input | StorageMode::Buffer)
    }
}

/// `TileEntityBatteryBase`'s settings: the mode without a redstone signal (`redLow`) and with one
/// (`redHigh`), and its charge priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageSettings {
    pub red_low: StorageMode,
    pub red_high: StorageMode,
    pub priority: Priority,
}

impl Default for StorageSettings {
    fn default() -> Self {
        StorageSettings {
            red_low: StorageMode::Input,
            red_high: StorageMode::Output,
            priority: Priority::Low,
        }
    }
}

impl StorageSettings {
    /// `getRelevantMode`: the mode with or without a signal at any of its blocks.
    pub fn mode(&self, powered: bool) -> StorageMode {
        if powered { self.red_high } else { self.red_low }
    }

    /// `updateEntity` puts a storage block at the lowest or highest priority back to low: only
    /// low, normal and high are a player's to pick.
    pub fn checked(mut self) -> StorageSettings {
        if matches!(self.priority, Priority::Lowest | Priority::Highest) {
            self.priority = Priority::Low;
        }
        self
    }

    /// `receiveControl`: the screen's button for the mode without a signal.
    pub fn cycle_low(&mut self) {
        self.red_low = self.red_low.next();
    }

    /// `receiveControl`: the screen's button for the mode with a signal.
    pub fn cycle_high(&mut self) {
        self.red_high = self.red_high.next();
    }

    /// `receiveControl`: low, normal, high, low again.
    pub fn cycle_priority(&mut self) {
        self.priority = match self.priority {
            Priority::Low => Priority::Normal,
            Priority::Normal => Priority::High,
            _ => Priority::Low,
        };
    }

    /// The socket screen's buttons ([`SOCKET_SCREEN`]): answers whether `id` is one.
    pub fn press(&mut self, id: i32) -> bool {
        match id {
            0 => self.cycle_low(),
            1 => self.cycle_high(),
            2 => self.cycle_priority(),
            _ => return false,
        }
        true
    }
}

/// The socket's menu data [`SOCKET_SCREEN`] reads: charge, capacity and change per second in four
/// slots each, then the mode without and with a signal, and the priority.
pub fn socket_data(charge: i64, max: i64, delta: i64, settings: StorageSettings) -> Vec<i32> {
    let mut data: Vec<i32> = [charge, max, delta].into_iter().flat_map(to_data).collect();
    data.extend([settings.red_low.index(), settings.red_high.index(), settings.priority.ordinal()]);
    data
}

/// `GUIBatterySocket`.
pub const SOCKET_SCREEN: Screen = Screen {
    texture: "ntm:gui/gui_battery_socket",
    size: [176.0, 181.0],
    slots: &[Some([35.0, 35.0])],
    inventory: [8.0, 99.0],
    title: Label { x: 88.0, y: 6.0, color: DARK },
    inventory_label: [8.0, 87.0],
    parts: &[
        Part::Bar { value: Value::Long(0), max: Value::Long(4), fill: Fill::Up, extra: 0.0, src: [176.0, 0.0], dst: [62.0, 17.0], size: [34.0, 52.0] },
        Part::Pick { value: Value::Data(12), base: 0, step: [0.0, 18.0], hide: None, src: [176.0, 52.0], dst: [106.0, 16.0], size: [18.0, 18.0] },
        Part::Pick { value: Value::Data(13), base: 0, step: [0.0, 18.0], hide: None, src: [176.0, 52.0], dst: [106.0, 52.0], size: [18.0, 18.0] },
        Part::Pick { value: Value::Data(14), base: 1, step: [0.0, 16.0], hide: None, src: [194.0, 52.0], dst: [125.0, 35.0], size: [16.0, 16.0] },
    ],
    tips: &[
        Tip { area: [62.0, 17.0, 34.0, 52.0], text: Text::Charge { charge: Value::Long(0), max: Value::Long(4), delta: Value::Long(8) } },
        Tip { area: [125.0, 35.0, 16.0, 16.0], text: Text::Priority { value: Value::Data(14) } },
    ],
    buttons: &[
        Button { area: [106.0, 16.0, 18.0, 18.0], id: 0 },
        Button { area: [106.0, 52.0, 18.0, 18.0], id: 1 },
        Button { area: [125.0, 35.0, 16.0, 16.0], id: 2 },
    ],
};

/// `TileEntityBatterySocket.getProviderSpeed`: the battery's discharge rate while it gives.
pub fn provider_speed(mode: StorageMode, battery: Option<&Battery>) -> i64 {
    match battery {
        Some(battery) if mode.provides() => battery.discharge_rate,
        _ => 0,
    }
}

/// `TileEntityBatterySocket.getReceiverSpeed`: the battery's charge rate while it takes.
pub fn receiver_speed(mode: StorageMode, battery: Option<&Battery>) -> i64 {
    match battery {
        Some(battery) if mode.receives() => battery.charge_rate,
        _ => 0,
    }
}

/// What `GUIBatterySocket` shows over the priority button (`battery.priority.*`, English).
pub fn priority_lines(priority: Priority) -> &'static [&'static str] {
    match priority {
        Priority::Low => &[
            "Charge Priority: Low",
            "Recommended for:",
            "Most use-cases where surplus",
            "energy needs to be stored without",
            "affecting machine performance",
        ],
        Priority::High => &[
            "Charge Priority: High",
            "Recommended for:",
            "Emergency buffers that",
            "always need to be full",
        ],
        _ => &[
            "Charge Priority: Normal",
            "Recommended for:",
            "Smaller power grids where",
            "priority doesn't matter",
        ],
    }
}

/// `getComparatorPower`: how full it is, 0 to 15.
pub fn comparator(power: i64, max_power: i64) -> i32 {
    let fraction = power as f64 / max_power.max(1) as f64 * 15.0;
    (fraction.round() as i32).clamp(0, 15)
}

/// The socket's last second of charge (`log`), for the change its screen shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChargeLog {
    pub log: [i64; 20],
}

impl ChargeLog {
    /// One tick: the average of the charge before and after it goes in, and the answer is the
    /// change since the oldest (`delta`, HE per second).
    pub fn push(&mut self, before: i64, after: i64) -> i64 {
        let average = before.wrapping_add(after) / 2;
        let delta = average.wrapping_sub(self.log[0]);
        self.log.rotate_left(1);
        self.log[19] = average;
        delta
    }
}

/// `BobMathUtil.getShortNumber`: `1.8M`, `72.0G`, as Java prints the double.
pub fn short_number(value: i64) -> String {
    const SUFFIXES: [(f64, &str); 6] = [(1e18, "E"), (1e15, "P"), (1e12, "T"), (1e9, "G"), (1e6, "M"), (1e3, "k")];
    let Some(&(scale, suffix)) = SUFFIXES.iter().find(|(scale, _)| value.unsigned_abs() as f64 >= *scale) else {
        return value.to_string();
    };
    let res = value as f64 / scale;
    // Java's Math.round: half up.
    let res = if res <= -100.0 { (res * 10.0 + 0.5).floor() / 10.0 } else { (res * 100.0 + 0.5).floor() / 100.0 };
    let mut text = res.to_string();
    if !text.contains('.') {
        text.push_str(".0");
    }
    text + suffix
}

/// A long in four menu data slots, which carry 16 bits each, low bits first.
pub fn to_data(value: i64) -> [i32; 4] {
    std::array::from_fn(|i| ((value >> (16 * i)) & 0xFFFF) as i32)
}

/// [`to_data`] back.
pub fn from_data(data: &[i32]) -> i64 {
    data.iter()
        .take(4)
        .enumerate()
        .fold(0, |value, (i, &part)| value | (i64::from(part) & 0xFFFF) << (16 * i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_sizes_follow_the_discharge_rate() {
        let redstone = BatteryPack::BatteryRedstone;
        assert_eq!((redstone.capacity(), redstone.charge_rate(), redstone.discharge_rate()), (1_800_000, 1_000, 100));
        let copper = BatteryPack::CapacitorCopper;
        assert_eq!((copper.capacity(), copper.charge_rate(), copper.discharge_rate()), (600_000, 1_000, 1_000));
        let quantum = BatteryPack::BatteryQuantum;
        assert_eq!((quantum.capacity(), quantum.charge_rate()), (72_000_000_000, 10_000_000));
        let spark = BatteryPack::CapacitorSpark;
        assert_eq!(spark.capacity(), 6_000_000_000);
        assert!(!quantum.is_capacitor() && copper.is_capacitor());
        assert_eq!(BatteryPack::from_name("battery_sodium"), Some(BatteryPack::BatterySodium));
    }

    #[test]
    fn charging_takes_the_least_of_power_rate_and_room() {
        let mut b = BatteryPack::BatteryRedstone.battery(1_799_500);
        assert_eq!(b.charge_from(5_000, 10_000), 4_500);
        assert_eq!(b.charge, 1_800_000);
        let mut b = BatteryPack::BatteryRedstone.battery(0);
        assert_eq!(b.charge_from(5_000, 10_000), 4_000);
        assert_eq!(b.charge, 1_000);
    }

    #[test]
    fn charging_keeps_javas_answers_for_odd_power() {
        let mut b = BatteryPack::BatteryRedstone.battery(0);
        assert_eq!(b.charge_from(-5, 10_000), 0);
        assert_eq!(b.charge_from(20_000, 10_000), 10_000);
        assert_eq!(b.charge, 0);
    }

    #[test]
    fn discharging_takes_the_least_of_room_rate_and_charge() {
        let mut b = BatteryPack::BatteryLead.battery(50_000);
        assert_eq!(b.discharge_into(9_900, 10_000), 10_000);
        assert_eq!(b.charge, 49_900);
        assert_eq!(b.discharge_into(0, 10_000), 1_000);
        let mut b = BatteryPack::BatteryLead.battery(300);
        assert_eq!(b.discharge_into(0, 10_000), 300);
        assert_eq!(b.charge, 0);
    }

    struct Packs;

    impl BatterySlot<(BatteryPack, i64)> for Packs {
        fn battery(&self, stack: &(BatteryPack, i64)) -> Option<Battery> {
            Some(stack.0.battery(stack.1))
        }
        fn set_charge(&self, stack: &mut (BatteryPack, i64), charge: i64) {
            stack.1 = charge;
        }
    }

    #[test]
    fn slots_change_the_stack_and_leave_an_empty_slot_alone() {
        let mut slot = Some((BatteryPack::BatteryLead, 0));
        assert_eq!(charge_slot(&Packs, &mut slot, 5_000, 10_000), 0);
        assert_eq!(slot, Some((BatteryPack::BatteryLead, 5_000)));
        assert_eq!(discharge_slot(&Packs, &mut slot, 0, 10_000), 1_000);
        assert_eq!(slot, Some((BatteryPack::BatteryLead, 4_000)));
        assert_eq!(discharge_slot(&Packs, &mut None, 7, 10_000), 7);
        assert_eq!(charge_slot(&Packs, &mut None, 20_000, 10_000), 10_000);
    }

    #[test]
    fn modes_follow_redstone_and_the_buttons_cycle() {
        let mut s = StorageSettings::default();
        assert_eq!((s.mode(false), s.mode(true)), (StorageMode::Input, StorageMode::Output));
        s.cycle_low();
        assert_eq!(s.red_low, StorageMode::Buffer);
        s.cycle_high();
        s.cycle_high();
        assert_eq!(s.red_high, StorageMode::Input);
        s.cycle_priority();
        s.cycle_priority();
        assert_eq!(s.priority, Priority::High);
        s.cycle_priority();
        assert_eq!(s.priority, Priority::Low);
        s.priority = Priority::Highest;
        assert_eq!(s.checked().priority, Priority::Low);
    }

    #[test]
    fn speeds_depend_on_the_mode() {
        let b = BatteryPack::BatteryLead.battery(0);
        assert_eq!(provider_speed(StorageMode::Buffer, Some(&b)), 1_000);
        assert_eq!(receiver_speed(StorageMode::Buffer, Some(&b)), 10_000);
        assert_eq!(provider_speed(StorageMode::Input, Some(&b)), 0);
        assert_eq!(receiver_speed(StorageMode::Output, Some(&b)), 0);
        assert_eq!(receiver_speed(StorageMode::Input, None), 0);
    }

    #[test]
    fn the_log_gives_the_change_over_twenty_ticks() {
        let mut log = ChargeLog::default();
        let mut delta = 0;
        for tick in 0..40 {
            delta = log.push(tick * 100, tick * 100 + 100);
        }
        assert_eq!(delta, 2_000);
        assert_eq!(comparator(50, 100), 8);
        assert_eq!(comparator(0, 0), 0);
    }

    #[test]
    fn short_numbers_print_as_java_does() {
        assert_eq!(short_number(999), "999");
        assert_eq!(short_number(1_800_000), "1.8M");
        assert_eq!(short_number(72_000_000_000), "72.0G");
        assert_eq!(short_number(12_345), "12.35k");
        assert_eq!(short_number(-150_000), "-150.0k");
    }

    #[test]
    fn longs_survive_sixteen_bit_data_slots() {
        for value in [0, 1, 65_535, 72_000_000_000, -1, i64::MAX] {
            assert_eq!(from_data(&to_data(value)), value);
        }
    }
}
