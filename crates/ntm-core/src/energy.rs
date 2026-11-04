//! The power network: a port of UNINOS (`UniNodespace`, `GenNode`, `NodeNet`, `TilePort`) and of
//! `PowerNetMK2` from NTM for 1.7.10. Power is in HE, a Java `long` there and an `i64` here.
//!
//! Nodes are the "souls" of cables and machine ports: they connect and form networks without the
//! block entities being looked at, so a network also runs through unloaded chunks. `H` names a
//! provider or receiver (a block position, for a block entity); [`EnergyHandlers`] reads and
//! changes the power of the one it names.
//!
//! Call [`Nodespace::update`] once per level at the start of each server tick, then tick the block
//! entities, as NTM does.
//!
//! Java arithmetic wraps, so sums here do too. NTM's sets have an arbitrary order; here handlers
//! go in `H`'s order, nets in creation order. Nothing but the choice of who pays for a rounding
//! error depends on either, and the original picks that at random.

use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Pos = [i32; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const ALL: [Dir; 6] = [
        Dir::Down,
        Dir::Up,
        Dir::North,
        Dir::South,
        Dir::West,
        Dir::East,
    ];

    pub fn offset(self) -> Pos {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }
}

/// `DirPos`: the block a node connects to, and the direction it leaves in to get there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    pub pos: Pos,
    pub dir: Dir,
}

impl Connection {
    /// The connection leaving `from` in direction `dir`.
    pub fn leaving(from: Pos, dir: Dir) -> Connection {
        let offset = dir.offset();
        Connection {
            pos: [
                from[0] + offset[0],
                from[1] + offset[1],
                from[2] + offset[2],
            ],
            dir,
        }
    }

    /// The block this connection leaves from.
    fn from(self) -> Pos {
        let offset = self.dir.offset();
        [
            self.pos[0] - offset[0],
            self.pos[1] - offset[1],
            self.pos[2] - offset[2],
        ]
    }
}

/// A connection on each of the six sides of `pos`: a cable, or a machine's `around` port.
pub fn around(pos: Pos) -> Vec<Connection> {
    Dir::ALL
        .iter()
        .map(|&dir| Connection::leaving(pos, dir))
        .collect()
}

/// `ConnectionPriority`: higher priorities are served first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Lowest,
    Low,
    Normal,
    High,
    Highest,
}

impl Priority {
    /// `ConnectionPriority.ordinal()`, as NTM saves it.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// `EnumUtil.grabEnumSafely`: an ordinal out of range is the first one.
    pub fn from_ordinal(ordinal: i32) -> Priority {
        match ordinal {
            1 => Priority::Low,
            2 => Priority::Normal,
            3 => Priority::High,
            4 => Priority::Highest,
            _ => Priority::Lowest,
        }
    }
}

const PRIORITIES: usize = 5;

/// `IEnergyHandlerMK2`, `IEnergyReceiverMK2` and `IEnergyProviderMK2`, for whatever `H` names.
pub trait EnergyHandlers<H: Copy> {
    fn power(&self, handler: H) -> i64;
    fn set_power(&mut self, handler: H, power: i64);
    fn max_power(&self, handler: H) -> i64;

    /// `NodeNet.isBadLink`: unloaded or removed. Such a handler leaves the network.
    fn is_bad_link(&self, handler: H) -> bool;

    fn receiver_speed(&self, handler: H) -> i64 {
        self.max_power(handler)
    }

    fn provider_speed(&self, handler: H) -> i64 {
        self.max_power(handler)
    }

    fn priority(&self, _handler: H) -> Priority {
        Priority::Normal
    }

    /// Adds `power`, answering what didn't fit.
    fn transfer_power(&mut self, handler: H, power: i64) -> i64 {
        let stored = self.power(handler);
        let max = self.max_power(handler);
        if power.wrapping_add(stored) <= max {
            self.set_power(handler, power.wrapping_add(stored));
            return 0;
        }
        let capacity = max.wrapping_sub(stored);
        self.set_power(handler, max);
        power.wrapping_sub(capacity)
    }

    /// Takes `power`, which must be at most the stored power: nothing here checks.
    fn use_power(&mut self, handler: H, power: i64) {
        let stored = self.power(handler);
        self.set_power(handler, stored.wrapping_sub(power));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetId(u64);

struct Node {
    positions: Vec<Pos>,
    connections: Vec<Connection>,
    net: Option<NetId>,
    recently_changed: bool,
}

/// `PowerNetMK2`. A destroyed net is removed at once, so "valid" is being in the map. That also
/// covers NTM's five-minute reaper: no net can be left without links but a destroyed one.
struct PowerNet<H> {
    links: BTreeSet<NodeId>,
    receivers: BTreeSet<H>,
    providers: BTreeSet<H>,
    energy_tracker: i64,
}

impl<H: Copy + Ord> PowerNet<H> {
    fn new() -> Self {
        PowerNet {
            links: BTreeSet::new(),
            receivers: BTreeSet::new(),
            providers: BTreeSet::new(),
            energy_tracker: 0,
        }
    }

    fn update<W: EnergyHandlers<H>>(
        &mut self,
        world: &mut W,
        rng: &mut impl FnMut(usize) -> usize,
    ) {
        if self.providers.is_empty() || self.receivers.is_empty() {
            return;
        }

        // Sum up available power.
        self.providers.retain(|&h| !world.is_bad_link(h));
        let mut providers = Vec::new();
        let mut total_power_in_network = 0i64;
        let mut power_available = 0i64;
        for &h in &self.providers {
            let src = world.power(h).min(world.provider_speed(h));
            if src > 0 {
                providers.push((h, src));
                power_available = power_available.wrapping_add(src);
                total_power_in_network = total_power_in_network.wrapping_add(world.power(h));
            }
        }

        // Sum up total demand, by priority.
        self.receivers.retain(|&h| !world.is_bad_link(h));
        let mut receivers: [Vec<(H, i64)>; PRIORITIES] = Default::default();
        let mut demand = [0i64; PRIORITIES];
        let mut total_demand = 0i64;
        for &h in &self.receivers {
            let rec = world
                .max_power(h)
                .wrapping_sub(world.power(h))
                .min(world.receiver_speed(h));
            if rec > 0 {
                let p = world.priority(h) as usize;
                receivers[p].push((h, rec));
                demand[p] = demand[p].wrapping_add(rec);
                total_demand = total_demand.wrapping_add(rec);
            }
        }

        let mut to_transfer = power_available.min(total_demand);
        let mut energy_used = 0i64;

        // Add power to receivers, highest priority first.
        for p in (0..PRIORITIES).rev() {
            for &(h, rec) in &receivers[p] {
                let weight = rec as f64 / demand[p] as f64;
                let to_send = (to_transfer as f64 * weight).max(0.0).min(rec as f64) as i64;
                let leftover = world.transfer_power(h, to_send);
                energy_used = energy_used.wrapping_add(to_send.wrapping_sub(leftover));
            }
            // As the original has it: `energy_used` is the running total, so what the higher
            // priorities took is subtracted again at each level below, empty ones included.
            to_transfer = to_transfer.wrapping_sub(energy_used);
        }

        // Take power from providers.
        let mut leftover = energy_used;
        for &(h, src) in &providers {
            let weight = src as f64 / power_available as f64;
            let to_use = (energy_used as f64 * weight).max(0.0) as i64;
            world.use_power(h, to_use);
            leftover = leftover.wrapping_sub(to_use);
        }

        // Rounding error: a random provider pays what is left.
        let mut iterations_left = 100;
        while iterations_left > 0 && leftover > 0 && !providers.is_empty() {
            iterations_left -= 1;
            let scapegoat = providers[rng(providers.len())].0;
            let to_use = leftover.min(world.power(scapegoat));
            world.use_power(scapegoat, to_use);
            leftover = leftover.wrapping_sub(to_use);
        }

        let mut power_available_after = 0i64;
        for &h in &self.providers {
            let src = world.power(h).max(0);
            if src > 0 {
                power_available_after = power_available_after.wrapping_add(src);
            }
        }
        self.energy_tracker = self
            .energy_tracker
            .wrapping_add(total_power_in_network.wrapping_sub(power_available_after));
    }
}

/// `UniNodeWorld` and `UniNodespace` for one level.
pub struct Nodespace<H> {
    nodes: BTreeMap<NodeId, Node>,
    at: HashMap<Pos, NodeId>,
    nets: BTreeMap<NetId, PowerNet<H>>,
    next_node: u64,
    next_net: u64,
}

impl<H> Default for Nodespace<H> {
    fn default() -> Self {
        Nodespace {
            nodes: BTreeMap::new(),
            at: HashMap::new(),
            nets: BTreeMap::new(),
            next_node: 0,
            next_net: 0,
        }
    }
}

impl<H: Copy + Ord> Nodespace<H> {
    pub fn node_at(&self, pos: Pos) -> Option<NodeId> {
        self.at.get(&pos).copied()
    }

    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    /// The node's network. `None` until the update after the node is made.
    pub fn net_of(&self, node: NodeId) -> Option<NetId> {
        self.nodes.get(&node).and_then(|n| n.net)
    }

    pub fn create_node(&mut self, positions: Vec<Pos>, connections: Vec<Connection>) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node += 1;
        for &pos in &positions {
            self.at.insert(pos, id);
        }
        self.nodes.insert(
            id,
            Node {
                positions,
                connections,
                net: None,
                recently_changed: true,
            },
        );
        id
    }

    /// Removes the node and destroys its whole network: the other nodes build new ones from
    /// their connections at the next update, and their ports subscribe again.
    pub fn destroy_node(&mut self, node: NodeId) {
        if let Some(net) = self.net_of(node) {
            self.destroy_net(net);
        }
        if let Some(removed) = self.nodes.remove(&node) {
            for pos in removed.positions {
                self.at.remove(&pos);
            }
        }
    }

    pub fn add_receiver(&mut self, net: NetId, handler: H) {
        if let Some(net) = self.nets.get_mut(&net) {
            net.receivers.insert(handler);
        }
    }

    pub fn add_provider(&mut self, net: NetId, handler: H) {
        if let Some(net) = self.nets.get_mut(&net) {
            net.providers.insert(handler);
        }
    }

    pub fn remove_handler(&mut self, net: NetId, handler: H) {
        self.remove_receiver(net, handler);
        self.remove_provider(net, handler);
    }

    pub fn remove_receiver(&mut self, net: NetId, handler: H) {
        if let Some(net) = self.nets.get_mut(&net) {
            net.receivers.remove(&handler);
        }
    }

    pub fn remove_provider(&mut self, net: NetId, handler: H) {
        if let Some(net) = self.nets.get_mut(&net) {
            net.providers.remove(&handler);
        }
    }

    /// The HE this network moved at its last update.
    pub fn energy_tracker(&self, net: NetId) -> Option<i64> {
        self.nets.get(&net).map(|n| n.energy_tracker)
    }

    /// What the network analysis tool shows: `(links, providers, receivers)`.
    pub fn net_sizes(&self, net: NetId) -> Option<(usize, usize, usize)> {
        self.nets
            .get(&net)
            .map(|n| (n.links.len(), n.providers.len(), n.receivers.len()))
    }

    /// `updateNodespace`: connects new and changed nodes, then runs every network. `rng(n)`
    /// answers a number below `n`.
    pub fn update<W: EnergyHandlers<H>>(
        &mut self,
        world: &mut W,
        rng: &mut impl FnMut(usize) -> usize,
    ) {
        let ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        for id in ids {
            let node = &self.nodes[&id];
            if node.net.is_none() || node.recently_changed {
                self.check_node_connection(id);
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.recently_changed = false;
                }
            }
        }

        for net in self.nets.values_mut() {
            net.energy_tracker = 0;
        }
        for net in self.nets.values_mut() {
            net.update(world, rng);
        }
    }

    fn check_node_connection(&mut self, id: NodeId) {
        for con in self.nodes[&id].connections.clone() {
            let Some(other) = self.node_at(con.pos) else {
                continue;
            };
            let other_net = self.nodes[&other].net;
            if other_net.is_some() && other_net == self.nodes[&id].net {
                continue;
            }
            if self.check_connection(other, con) {
                self.connect_to_node(id, other);
            }
        }
        if self.nodes[&id].net.is_none() {
            let net = NetId(self.next_net);
            self.next_net += 1;
            self.nets.insert(net, PowerNet::new());
            self.join_link(net, id);
        }
    }

    /// Whether `connects_to` has a connection coming back the way `from` arrives.
    fn check_connection(&self, connects_to: NodeId, from: Connection) -> bool {
        self.nodes[&connects_to]
            .connections
            .iter()
            .any(|rev| rev.from() == from.pos && rev.dir == from.dir.opposite())
    }

    fn connect_to_node(&mut self, origin: NodeId, connection: NodeId) {
        match (self.nodes[&origin].net, self.nodes[&connection].net) {
            (Some(a), Some(b)) => {
                if self.nets[&a].links.len() > self.nets[&b].links.len() {
                    self.join_networks(a, b);
                } else {
                    self.join_networks(b, a);
                }
            }
            (None, Some(b)) => self.join_link(b, origin),
            (Some(a), None) => self.join_link(a, connection),
            (None, None) => {}
        }
    }

    fn join_link(&mut self, net: NetId, node: NodeId) {
        if let Some(net) = self.nets.get_mut(&net) {
            net.links.insert(node);
        }
        self.set_net(node, Some(net));
    }

    fn set_net(&mut self, node: NodeId, net: Option<NetId>) {
        if let Some(node) = self.nodes.get_mut(&node) {
            node.net = net;
            node.recently_changed = true;
        }
    }

    /// Moves everything in `from` into `into`.
    fn join_networks(&mut self, into: NetId, from: NetId) {
        if into == from {
            return;
        }
        let Some(old) = self.nets.remove(&from) else {
            return;
        };
        for link in old.links {
            self.join_link(into, link);
        }
        if let Some(net) = self.nets.get_mut(&into) {
            net.receivers.extend(old.receivers);
            net.providers.extend(old.providers);
        }
    }

    fn destroy_net(&mut self, net: NetId) {
        if let Some(old) = self.nets.remove(&net) {
            for link in old.links {
                self.set_net(link, None);
            }
        }
    }
}

/// `TilePort`: a machine's node, and its subscription to whatever network the node is in.
///
/// Each tick a machine calls [`Port::update`], then [`Port::check_subscribe`] or
/// [`Port::check_provide`]. It calls [`Port::disable_if_present`] when it unloads or is removed.
pub struct Port {
    positions: Vec<Pos>,
    connections: Vec<Connection>,
    node: Option<NodeId>,
    prev_net: Option<NetId>,
    time_since_network_change: u32,
}

impl Port {
    pub fn new(positions: Vec<Pos>, connections: Vec<Connection>) -> Port {
        Port {
            positions,
            connections,
            node: None,
            prev_net: None,
            time_since_network_change: 0,
        }
    }

    fn net<H: Copy + Ord>(&self, space: &Nodespace<H>) -> Option<NetId> {
        self.node.and_then(|node| space.net_of(node))
    }

    pub fn update<H: Copy + Ord>(&mut self, space: &mut Nodespace<H>, owner: H) {
        let current = self.net(space);
        if current != self.prev_net {
            self.time_since_network_change = 0;
            if let Some(prev) = self.prev_net {
                space.remove_handler(prev, owner);
            }
        } else {
            self.time_since_network_change += 1;
        }
        self.prev_net = current;
        self.enable_if_missing(space);
    }

    fn enable_if_missing<H: Copy + Ord>(&mut self, space: &mut Nodespace<H>) {
        if self.node.is_some_and(|node| space.contains(node)) {
            return;
        }
        let node = self.positions.first().and_then(|&pos| space.node_at(pos));
        self.node = Some(node.unwrap_or_else(|| {
            space.create_node(self.positions.clone(), self.connections.clone())
        }));
    }

    pub fn disable_if_present<H: Copy + Ord>(&mut self, space: &mut Nodespace<H>) {
        if let Some(node) = self.node.take() {
            space.destroy_node(node);
        }
    }

    pub fn check_subscribe<H: Copy + Ord>(&self, space: &mut Nodespace<H>, owner: H) {
        if self.time_since_network_change < 2
            && let Some(net) = self.net(space)
        {
            space.add_receiver(net, owner);
        }
    }

    pub fn check_provide<H: Copy + Ord>(&self, space: &mut Nodespace<H>, owner: H) {
        if self.time_since_network_change < 2
            && let Some(net) = self.net(space)
        {
            space.add_provider(net, owner);
        }
    }

    /// `TileEntityBatteryBase.updateEntity`: a storage block's node is a conductor like a cable's,
    /// and every tick the block is on its network as a provider, a receiver, both or neither.
    pub fn join<H: Copy + Ord>(&self, space: &mut Nodespace<H>, owner: H, provide: bool, receive: bool) {
        let Some(net) = self.net(space) else {
            return;
        };
        if provide {
            space.add_provider(net, owner);
        } else {
            space.remove_provider(net, owner);
        }
        if receive {
            space.add_receiver(net, owner);
        } else {
            space.remove_receiver(net, owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Machine {
        power: i64,
        max: i64,
        receiver_speed: i64,
        provider_speed: i64,
        priority: Priority,
        bad: bool,
    }

    fn machine(power: i64, max: i64) -> Machine {
        Machine {
            power,
            max,
            receiver_speed: max,
            provider_speed: max,
            priority: Priority::Normal,
            bad: false,
        }
    }

    #[derive(Default)]
    struct World {
        machines: BTreeMap<u32, Machine>,
    }

    impl World {
        fn power(&self, id: u32) -> i64 {
            self.machines[&id].power
        }
    }

    impl EnergyHandlers<u32> for World {
        fn power(&self, h: u32) -> i64 {
            self.machines[&h].power
        }
        fn set_power(&mut self, h: u32, power: i64) {
            self.machines.get_mut(&h).unwrap().power = power;
        }
        fn max_power(&self, h: u32) -> i64 {
            self.machines[&h].max
        }
        fn is_bad_link(&self, h: u32) -> bool {
            self.machines[&h].bad
        }
        fn receiver_speed(&self, h: u32) -> i64 {
            self.machines[&h].receiver_speed
        }
        fn provider_speed(&self, h: u32) -> i64 {
            self.machines[&h].provider_speed
        }
        fn priority(&self, h: u32) -> Priority {
            self.machines[&h].priority
        }
    }

    fn no_rng(_: usize) -> usize {
        panic!("no rounding error expected")
    }

    /// One net with the machines already subscribed, distributing with no graph around it.
    fn distribute(world: &mut World, providers: &[u32], receivers: &[u32]) -> i64 {
        distribute_with(world, providers, receivers, &mut no_rng)
    }

    fn distribute_with(
        world: &mut World,
        providers: &[u32],
        receivers: &[u32],
        rng: &mut impl FnMut(usize) -> usize,
    ) -> i64 {
        let mut net = PowerNet::new();
        net.providers.extend(providers);
        net.receivers.extend(receivers);
        net.update(world, rng);
        net.energy_tracker
    }

    fn world(machines: &[(u32, Machine)]) -> World {
        World {
            machines: machines.iter().cloned().collect(),
        }
    }

    #[test]
    fn moves_what_the_receiver_can_take() {
        let mut w = world(&[(1, machine(1000, 1000)), (2, machine(9700, 10000))]);
        assert_eq!(distribute(&mut w, &[1], &[2]), 300);
        assert_eq!(w.power(1), 700);
        assert_eq!(w.power(2), 10000);
    }

    #[test]
    fn moves_what_the_provider_has() {
        let mut w = world(&[(1, machine(40, 1000)), (2, machine(0, 10000))]);
        assert_eq!(distribute(&mut w, &[1], &[2]), 40);
        assert_eq!(w.power(1), 0);
        assert_eq!(w.power(2), 40);
    }

    #[test]
    fn speeds_limit_both_sides() {
        let mut slow_out = machine(1000, 1000);
        slow_out.provider_speed = 25;
        let mut w = world(&[(1, slow_out), (2, machine(0, 10000))]);
        distribute(&mut w, &[1], &[2]);
        assert_eq!((w.power(1), w.power(2)), (975, 25));

        let mut slow_in = machine(0, 10000);
        slow_in.receiver_speed = 7;
        let mut w = world(&[(1, machine(1000, 1000)), (2, slow_in)]);
        distribute(&mut w, &[1], &[2]);
        assert_eq!((w.power(1), w.power(2)), (993, 7));
    }

    #[test]
    fn nothing_moves_without_both_sides() {
        let mut w = world(&[(1, machine(1000, 1000))]);
        assert_eq!(distribute(&mut w, &[1], &[]), 0);
        assert_eq!(w.power(1), 1000);
    }

    #[test]
    fn receivers_share_by_demand_and_round_down() {
        // 100 HE for demands of 100 and 300: weights 1/4 and 3/4.
        let mut w = world(&[
            (1, machine(100, 100)),
            (2, machine(0, 100)),
            (3, machine(0, 300)),
        ]);
        distribute(&mut w, &[1], &[2, 3]);
        assert_eq!((w.power(2), w.power(3)), (25, 75));
        assert_eq!(w.power(1), 0);

        // 2 HE for demands of 1 and 2: 2/3 rounds to 0, 4/3 to 1, and the provider pays only 1.
        let mut w = world(&[
            (1, machine(2, 100)),
            (2, machine(99, 100)),
            (3, machine(98, 100)),
        ]);
        distribute(&mut w, &[1], &[2, 3]);
        assert_eq!((w.power(2), w.power(3)), (99, 99));
        assert_eq!(w.power(1), 1);
    }

    #[test]
    fn a_random_provider_pays_the_rounding_error() {
        // 2 HE used, three providers of equal weight: 2/3 rounds to 0 for each.
        let mut w = world(&[
            (1, machine(1, 100)),
            (2, machine(1, 100)),
            (3, machine(1, 100)),
            (4, machine(98, 100)),
        ]);
        let mut picks = vec![2, 0].into_iter();
        let tracked = distribute_with(&mut w, &[1, 2, 3], &[4], &mut |n| {
            assert_eq!(n, 3);
            picks.next().unwrap()
        });
        assert_eq!((w.power(1), w.power(2), w.power(3)), (0, 1, 0));
        assert_eq!(w.power(4), 100);
        assert_eq!(tracked, 2);
    }

    #[test]
    fn the_rounding_error_search_gives_up_after_100_tries() {
        // Taking anything empties these providers, so 1 HE stays unpaid and no pick can take it.
        struct Draining(World);
        impl EnergyHandlers<u32> for Draining {
            fn power(&self, h: u32) -> i64 {
                self.0.power(h)
            }
            fn set_power(&mut self, h: u32, p: i64) {
                self.0.set_power(h, p)
            }
            fn max_power(&self, h: u32) -> i64 {
                self.0.max_power(h)
            }
            fn is_bad_link(&self, _: u32) -> bool {
                false
            }
            fn use_power(&mut self, h: u32, _: i64) {
                self.0.set_power(h, 0);
            }
        }
        // 5 HE used, two providers of equal weight: each pays 2 of 2.5.
        let mut w = Draining(world(&[
            (1, machine(3, 100)),
            (2, machine(3, 100)),
            (3, machine(95, 100)),
        ]));
        let mut calls = 0;
        let mut net = PowerNet::new();
        net.providers.extend([1, 2]);
        net.receivers.insert(3);
        net.update(&mut w, &mut |_| {
            calls += 1;
            0
        });
        assert_eq!(calls, 100);
    }

    fn prioritized(power: i64, max: i64, priority: Priority) -> Machine {
        Machine {
            priority,
            ..machine(power, max)
        }
    }

    #[test]
    fn higher_priorities_are_served_first() {
        let mut w = world(&[
            (1, machine(10, 100)),
            (2, prioritized(0, 100, Priority::Low)),
            (3, prioritized(0, 100, Priority::High)),
        ]);
        distribute(&mut w, &[1], &[2, 3]);
        assert_eq!((w.power(2), w.power(3)), (0, 10));
    }

    #[test]
    fn what_higher_priorities_took_is_subtracted_again_at_each_level() {
        // The original's running total. From 100 HE: High takes 10, leaving 90. Normal takes 30,
        // and 40 comes off, leaving 50, not 60. Low gets 50, so 10 HE stay though it wants more.
        let mut w = world(&[
            (1, machine(100, 1000)),
            (2, prioritized(0, 10, Priority::High)),
            (3, prioritized(0, 30, Priority::Normal)),
            (4, prioritized(0, 1000, Priority::Low)),
        ]);
        assert_eq!(distribute(&mut w, &[1], &[2, 3, 4]), 90);
        assert_eq!((w.power(2), w.power(3), w.power(4)), (10, 30, 50));
        assert_eq!(w.power(1), 10);
    }

    #[test]
    fn an_empty_level_subtracts_too() {
        // Highest takes 10 of 100, leaving 90. Empty High takes off 10 more, so Normal gets 80.
        let mut w = world(&[
            (1, machine(100, 1000)),
            (2, prioritized(0, 10, Priority::Highest)),
            (3, prioritized(0, 1000, Priority::Normal)),
        ]);
        distribute(&mut w, &[1], &[2, 3]);
        assert_eq!((w.power(2), w.power(3)), (10, 80));
        assert_eq!(w.power(1), 10);
    }

    #[test]
    fn what_a_receiver_refuses_stays_with_the_provider() {
        struct Refusing(World);
        impl EnergyHandlers<u32> for Refusing {
            fn power(&self, h: u32) -> i64 {
                self.0.power(h)
            }
            fn set_power(&mut self, h: u32, p: i64) {
                self.0.set_power(h, p)
            }
            fn max_power(&self, h: u32) -> i64 {
                self.0.max_power(h)
            }
            fn is_bad_link(&self, _: u32) -> bool {
                false
            }
            fn transfer_power(&mut self, h: u32, power: i64) -> i64 {
                self.0.set_power(h, power / 2);
                power - power / 2
            }
        }
        let mut w = Refusing(world(&[(1, machine(100, 100)), (2, machine(0, 100))]));
        let mut net = PowerNet::new();
        net.providers.insert(1);
        net.receivers.insert(2);
        net.update(&mut w, &mut no_rng);
        assert_eq!((w.0.power(1), w.0.power(2)), (50, 50));
    }

    #[test]
    fn bad_links_leave_the_network() {
        let mut gone = machine(1000, 1000);
        gone.bad = true;
        let mut w = world(&[(1, gone), (2, machine(5, 100)), (3, machine(0, 100))]);
        let mut net = PowerNet::new();
        net.providers.extend([1, 2]);
        net.receivers.insert(3);
        net.update(&mut w, &mut no_rng);
        assert!(!net.providers.contains(&1));
        assert_eq!((w.power(1), w.power(2), w.power(3)), (1000, 0, 5));
    }

    #[test]
    fn the_tracker_counts_only_providers_that_could_send_before() {
        // A provider with speed 0 isn't in "before" but is in "after": the original's sum.
        let mut stuck = machine(30, 100);
        stuck.provider_speed = 0;
        let mut w = world(&[(1, machine(50, 100)), (2, stuck), (3, machine(90, 100))]);
        assert_eq!(distribute(&mut w, &[1, 2], &[3]), -20);
        assert_eq!((w.power(1), w.power(2), w.power(3)), (40, 30, 100));
    }

    fn cable(space: &mut Nodespace<u32>, x: i32) -> NodeId {
        let pos = [x, 0, 0];
        space.create_node(vec![pos], around(pos))
    }

    fn step(space: &mut Nodespace<u32>, w: &mut World) {
        space.update(w, &mut no_rng);
    }

    #[test]
    fn a_new_node_has_a_network_after_one_update() {
        let mut space = Nodespace::default();
        let a = cable(&mut space, 0);
        assert_eq!(space.net_of(a), None);
        step(&mut space, &mut World::default());
        assert!(space.net_of(a).is_some());
    }

    #[test]
    fn touching_cables_share_a_network_and_others_dont() {
        let mut space = Nodespace::default();
        let a = cable(&mut space, 0);
        let b = cable(&mut space, 1);
        let far = cable(&mut space, 5);
        let mut w = World::default();
        step(&mut space, &mut w);
        assert_eq!(space.net_of(a), space.net_of(b));
        assert_ne!(space.net_of(a), space.net_of(far));
    }

    #[test]
    fn a_bridge_joins_two_networks_and_keeps_their_machines() {
        let mut space = Nodespace::default();
        let a = cable(&mut space, 0);
        let b = cable(&mut space, 2);
        let mut w = World::default();
        step(&mut space, &mut w);
        space.add_provider(space.net_of(a).unwrap(), 1);
        space.add_receiver(space.net_of(b).unwrap(), 2);
        w.machines.insert(1, machine(100, 100));
        w.machines.insert(2, machine(0, 100));
        step(&mut space, &mut w);
        assert_eq!((w.power(1), w.power(2)), (100, 0));

        let bridge = cable(&mut space, 1);
        step(&mut space, &mut w);
        assert_eq!(space.net_of(a), space.net_of(b));
        assert_eq!(space.net_of(a), space.net_of(bridge));
        assert_eq!((w.power(1), w.power(2)), (0, 100));
    }

    #[test]
    fn breaking_a_cable_splits_the_network() {
        let mut space = Nodespace::default();
        let a = cable(&mut space, 0);
        let middle = cable(&mut space, 1);
        let c = cable(&mut space, 2);
        let mut w = World::default();
        step(&mut space, &mut w);
        let before = space.net_of(a).unwrap();

        space.destroy_node(middle);
        assert_eq!(space.net_of(a), None);
        assert_eq!(space.net_of(c), None);
        assert_eq!(space.node_at([1, 0, 0]), None);
        assert!(space.energy_tracker(before).is_none());

        step(&mut space, &mut w);
        assert!(space.net_of(a).is_some());
        assert!(space.net_of(c).is_some());
        assert_ne!(space.net_of(a), space.net_of(c));
    }

    #[test]
    fn a_connection_needs_one_coming_back() {
        let mut space = Nodespace::default();
        // East-facing only, at x = 0, reaching x = 1: x = 1 faces only east as well.
        let one_way = space.create_node(
            vec![[0, 0, 0]],
            vec![Connection::leaving([0, 0, 0], Dir::East)],
        );
        let away = space.create_node(
            vec![[1, 0, 0]],
            vec![Connection::leaving([1, 0, 0], Dir::East)],
        );
        step(&mut space, &mut World::default());
        assert_ne!(space.net_of(one_way), space.net_of(away));
    }

    #[test]
    fn a_node_can_fill_several_blocks() {
        let mut space = Nodespace::default();
        let big = space.create_node(
            vec![[0, 0, 0], [0, 1, 0]],
            vec![Connection::leaving([0, 1, 0], Dir::Up)],
        );
        let top = space.create_node(vec![[0, 2, 0]], around([0, 2, 0]));
        step(&mut space, &mut World::default());
        assert_eq!(space.net_of(big), space.net_of(top));
        assert_eq!(space.node_at([0, 0, 0]), Some(big));
        space.destroy_node(big);
        assert_eq!(space.node_at([0, 1, 0]), None);
    }

    /// A generator at x = 0 and a machine at x = 2, ticking as block entities do after the
    /// nodespace update at the start of the server tick.
    struct Line {
        space: Nodespace<u32>,
        world: World,
        generator: Port,
        machine: Port,
    }

    impl Line {
        fn new() -> Line {
            let world = world(&[(1, machine(1000, 1000)), (2, machine(0, 100))]);
            let mut space = Nodespace::default();
            cable(&mut space, 1);
            Line {
                space,
                world,
                generator: Port::new(vec![[0, 0, 0]], around([0, 0, 0])),
                machine: Port::new(vec![[2, 0, 0]], around([2, 0, 0])),
            }
        }

        fn tick(&mut self) {
            self.space.update(&mut self.world, &mut no_rng);
            self.generator.update(&mut self.space, 1);
            self.generator.check_provide(&mut self.space, 1);
            self.machine.update(&mut self.space, 2);
            self.machine.check_subscribe(&mut self.space, 2);
        }
    }

    #[test]
    fn ports_subscribe_and_power_flows_on_the_third_tick() {
        let mut line = Line::new();
        // Tick 1: the ports make their nodes, after the update, so nothing has a net yet.
        line.tick();
        assert_eq!(line.world.power(2), 0);
        // Tick 2: the nodes join the cable's net in the update, then the ports subscribe.
        line.tick();
        assert_eq!(line.world.power(2), 0);
        // Tick 3: the net runs with both ends subscribed.
        line.tick();
        assert_eq!(line.world.power(2), 100);
        assert_eq!(line.world.power(1), 900);
    }

    #[test]
    fn a_port_subscribes_again_after_its_network_is_rebuilt() {
        let mut line = Line::new();
        for _ in 0..5 {
            line.tick();
        }
        line.world.set_power(2, 0);
        let cable = line.space.node_at([1, 0, 0]).unwrap();
        line.space.destroy_node(cable);
        // Long past the two ticks a port subscribes for on its own.
        for _ in 0..5 {
            line.tick();
        }
        assert_eq!(line.world.power(2), 0, "no cable, no power");

        let pos = [1, 0, 0];
        line.space.create_node(vec![pos], around(pos));
        for _ in 0..5 {
            line.tick();
        }
        assert_eq!(line.world.power(2), 100);
    }

    #[test]
    fn a_storage_block_takes_as_input_and_gives_as_output() {
        // A generator, a storage block and a full machine in a row, ports touching.
        let mut storage = machine(0, 10_000);
        (storage.receiver_speed, storage.provider_speed, storage.priority) = (300, 200, Priority::Low);
        let mut w = world(&[(1, machine(1_000, 1_000)), (2, storage), (3, machine(1_000, 1_000))]);
        let mut space = Nodespace::default();
        let mut ports: Vec<Port> = (0..3).map(|x| Port::new(vec![[x, 0, 0]], around([x, 0, 0]))).collect();
        let mut tick = |space: &mut Nodespace<u32>, w: &mut World, provide, receive| {
            for (h, port) in (1..).zip(ports.iter_mut()) {
                port.update(space, h);
            }
            ports[0].check_provide(space, 1);
            ports[1].join(space, 2, provide, receive);
            ports[2].check_subscribe(space, 3);
            space.update(w, &mut no_rng);
        };
        for _ in 0..3 {
            tick(&mut space, &mut w, false, true);
        }
        assert_eq!((w.power(1), w.power(2)), (400, 600));

        w.set_power(1, 0);
        w.set_power(3, 0);
        tick(&mut space, &mut w, true, false);
        assert_eq!((w.power(2), w.power(3)), (400, 200));
    }

    #[test]
    fn a_disabled_port_takes_its_network_down_and_stops_receiving() {
        let mut line = Line::new();
        for _ in 0..5 {
            line.tick();
        }
        line.world.set_power(2, 0);
        line.machine.disable_if_present(&mut line.space);
        assert_eq!(line.space.node_at([2, 0, 0]), None);
        line.space.update(&mut line.world, &mut no_rng);
        assert_eq!(line.world.power(2), 0);
    }
}
