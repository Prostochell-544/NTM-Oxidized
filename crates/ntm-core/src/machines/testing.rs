//! A small world for the machines' tests: stacks by name and a few recipes.

use super::MachineWorld;
use crate::battery::{Battery, BatteryPack, BatterySlot};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub item: &'static str,
    pub count: u32,
    pub damage: u32,
}

pub fn stack(item: &'static str, count: u32) -> Option<Stack> {
    Some(Stack { item, count, damage: 0 })
}

#[derive(Default)]
pub struct World {
    pub power: i64,
    pub neighbors: HashMap<[i32; 3], &'static str>,
}

impl BatterySlot<Stack> for World {
    // A lead-acid battery keeps its charge in `damage` here.
    fn battery(&self, stack: &Stack) -> Option<Battery> {
        (stack.item == "battery").then(|| BatteryPack::BatteryLead.battery(i64::from(stack.damage)))
    }
    fn set_charge(&self, stack: &mut Stack, charge: i64) {
        stack.damage = charge as u32;
    }
}

impl MachineWorld<Stack> for World {
    fn id<'s>(&self, stack: &'s Stack) -> &'s str {
        stack.item
    }
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
    fn damage(&self, stack: &Stack) -> (u32, u32) {
        let max = match stack.item {
            "steel" => 200,
            "stamp_plate" => 3,
            _ => 0,
        };
        (stack.damage, max)
    }
    fn set_damage(&self, stack: &mut Stack, damage: u32) {
        stack.damage = damage;
    }
    fn power(&self) -> i64 {
        self.power
    }
    fn set_power(&mut self, power: i64) {
        self.power = power;
    }
    fn neighbor(&self, offset: [i32; 3]) -> String {
        self.neighbors.get(&offset).copied().unwrap_or("minecraft:air").to_owned()
    }
    fn burn_time(&self, stack: &Stack) -> i32 {
        match stack.item {
            "coal" => 1600,
            "lava" => 20_000,
            _ => 0,
        }
    }
    fn remainder(&self, used: &Stack) -> Option<Stack> {
        (used.item == "lava").then(|| stack("bucket", 1)).flatten()
    }
    fn blade_damage(&self, stack: &Stack) -> Option<(u32, u32)> {
        matches!(stack.item, "steel" | "desh").then(|| self.damage(stack))
    }
    fn stamp(&self, stack: &Stack) -> Option<&'static str> {
        (stack.item == "stamp_plate").then_some("plate")
    }
    fn shredder_result(&self, input: &Stack) -> Option<Stack> {
        match input.item {
            "ore" => stack("powder", 2),
            "ingot" => stack("powder", 1),
            _ => stack("scrap", 1),
        }
    }
    fn blast_result(&self, a: &Stack, b: &Stack) -> Option<(u32, u32, Stack)> {
        match (a.item, b.item) {
            ("iron", "coal") => Some((1, 1, stack("steel_ingot", 1)?)),
            ("coal", "iron") => Some((1, 1, stack("steel_ingot", 1)?)),
            ("copper", "redstone") => Some((1, 2, stack("red_copper", 2)?)),
            _ => None,
        }
    }
    fn blast_fuel(&self, stack: &Stack) -> i32 {
        match stack.item {
            "coal" => 200,
            "lava" => 12_800,
            _ => 0,
        }
    }
    fn press_result(&self, stamp: &str, input: &Stack) -> Option<Stack> {
        (stamp == "plate" && input.item == "ingot").then(|| stack("plate", 1)).flatten()
    }
}

/// Fields in a map, as a block entity's NBT keeps them.
#[derive(Default)]
pub struct Fields(pub HashMap<String, i32>);

impl super::Nbt for Fields {
    fn int(&self, key: &str) -> i32 {
        self.0.get(key).copied().unwrap_or(0)
    }
    fn set_int(&mut self, key: &str, value: i32) {
        self.0.insert(key.to_owned(), value);
    }
}
