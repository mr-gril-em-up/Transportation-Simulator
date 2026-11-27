//! Road network as a compressed adjacency (CSR) graph, Dijkstra shortest-path
//! trees, car skims and all-or-nothing loading.

use crate::params::lane_capacity_vph;
use crate::schema::ScenarioInput;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

pub const NONE: u32 = u32::MAX;

/// One directed traversal of one input road link.
#[derive(Debug, Clone)]
pub struct Member {
    /// Index into `ScenarioInput::road_links`.
    pub link: u32,
    /// Dense start node (traversal direction).
    pub from: u32,
    /// Dense end node.
    pub to: u32,
    pub len_km: f64,
    pub t0_min: f64,
    pub cap_vph: f64,
    pub toll: f64,
}

/// A directed graph edge: one input link traversal, or a chain of them through
/// contracted degree-2 nodes (same volume on every member).
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    pub len_km: f64,
    /// Free-flow time, the sum over members.
    pub t0_min: f64,
    pub toll: f64,
    /// `members[m_lo..m_hi]` of the graph, in travel order.
    pub m_lo: u32,
    pub m_hi: u32,
}

/// Road graph for shortest paths. Built from the input links with two
/// exact simplifications (SIM-2):
/// - dead-end trees with no zone node are dropped (no shortest path between
///   zone nodes can enter them);
/// - chains through nodes with exactly two neighbors and no zone are merged
///   into one edge whose BPR time is the sum of its members' BPR times.
///
/// Node numbering stays the input order (dropped nodes just have no edges).
pub struct RoadGraph {
    pub n_nodes: usize,
    pub index: HashMap<u32, u32>,
    /// `edges[offsets[v]..offsets[v+1]]` leave node `v`.
    pub offsets: Vec<u32>,
    pub edges: Vec<Edge>,
    /// `edges[e].to`, stored densely: the Dijkstra inner loop reads only this
    /// and the time array, so keeping it compact halves cache misses.
    pub to: Vec<u32>,
    /// Link traversals behind each edge (see `Edge::m_lo`).
    pub members: Vec<Member>,
    /// Link traversals in dropped dead ends: never loaded, reported at free flow.
    pub dead: Vec<Member>,
}

impl RoadGraph {
    /// Build with dead-end pruning and chain contraction around `keep` nodes
    /// (zone nodes, by input node id).
    pub fn build(input: &ScenarioInput) -> RoadGraph {
        let keep_ids: Vec<u32> = input.zones.iter().map(|z| z.road_node).collect();
        Self::build_keeping(input, &keep_ids, true)
    }

    /// `simplify = false` keeps one edge per link traversal (for tests).
    pub fn build_keeping(input: &ScenarioInput, keep_ids: &[u32], simplify: bool) -> RoadGraph {
        let index: HashMap<u32, u32> = input
            .road_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i as u32))
            .collect();
        let n = input.road_nodes.len();
        // Directed traversals, and per node the traversals leaving it.
        let mut trav: Vec<Member> = Vec::with_capacity(input.road_links.len() * 2);
        for (li, l) in input.road_links.iter().enumerate() {
            let (a, b) = (index[&l.from], index[&l.to]);
            let len_km = l.length_m / 1000.0;
            let cap = l
                .capacity_vph
                .unwrap_or(lane_capacity_vph(l.speed_kph) * l.lanes.max(1) as f64);
            let mk = |from, to| Member {
                link: li as u32,
                from,
                to,
                len_km,
                t0_min: len_km / l.speed_kph * 60.0,
                cap_vph: cap.max(1.0),
                toll: l.toll,
            };
            trav.push(mk(a, b));
            if !l.oneway {
                trav.push(mk(b, a));
            }
        }
        let mut out: Vec<Vec<u32>> = vec![Vec::new(); n];
        // Undirected incidence: link indices touching each node.
        let mut inc: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (t, m) in trav.iter().enumerate() {
            out[m.from as usize].push(t as u32);
        }
        for (li, l) in input.road_links.iter().enumerate() {
            inc[index[&l.from] as usize].push(li as u32);
            inc[index[&l.to] as usize].push(li as u32);
        }
        let mut keep = vec![!simplify; n];
        for id in keep_ids {
            if let Some(&v) = index.get(id) {
                keep[v as usize] = true;
            }
        }
        let other = |li: u32, v: usize| -> usize {
            let l = &input.road_links[li as usize];
            let a = index[&l.from] as usize;
            if a == v {
                index[&l.to] as usize
            } else {
                a
            }
        };

        // 1. Prune dead ends: repeatedly drop unkept nodes with < 2 distinct neighbors.
        let mut alive_link = vec![true; input.road_links.len()];
        let mut alive_node = vec![true; n];
        let neighbors = |v: usize, alive_link: &[bool]| -> Vec<usize> {
            let mut ns: Vec<usize> = inc[v]
                .iter()
                .filter(|&&li| alive_link[li as usize])
                .map(|&li| other(li, v))
                .collect();
            ns.sort_unstable();
            ns.dedup();
            ns
        };
        let mut stack: Vec<usize> = (0..n).filter(|&v| !keep[v]).collect();
        while let Some(v) = stack.pop() {
            if !alive_node[v] || keep[v] {
                continue;
            }
            let ns = neighbors(v, &alive_link);
            if ns.len() < 2 {
                alive_node[v] = false;
                for &li in &inc[v] {
                    alive_link[li as usize] = false;
                }
                stack.extend(ns);
            }
        }

        // 2. Interior nodes: unkept, alive, exactly two neighbors over exactly two links.
        let interior: Vec<bool> = (0..n)
            .map(|v| {
                !keep[v] && alive_node[v] && {
                    let live: Vec<u32> = inc[v]
                        .iter()
                        .copied()
                        .filter(|&li| alive_link[li as usize])
                        .collect();
                    live.len() == 2 && other(live[0], v) != other(live[1], v)
                }
            })
            .collect();

        // 3. Walk chains from every live traversal leaving a non-interior node.
        let mut edges = Vec::new();
        let mut members = Vec::new();
        let mut dead = Vec::new();
        for (t0, m) in trav.iter().enumerate() {
            if !alive_link[m.link as usize] {
                dead.push(m.clone());
                continue;
            }
            if interior[m.from as usize] {
                continue;
            }
            let lo = members.len();
            let mut cur = t0 as u32;
            let ok = loop {
                let c = &trav[cur as usize];
                members.push(c.clone());
                let v = c.to as usize;
                if !interior[v] {
                    break true;
                }
                // Continue along the other link of the interior node, if it can
                // be driven in this direction.
                match out[v].iter().find(|&&t| {
                    let x = &trav[t as usize];
                    x.link != c.link && alive_link[x.link as usize]
                }) {
                    Some(&t) => cur = t,
                    None => break false,
                }
            };
            let chain = &members[lo..];
            let (from, to) = (chain[0].from, chain[chain.len() - 1].to);
            if !ok || from == to {
                // Dead-end oneway chain or a loop back to its start: unusable.
                dead.extend(members.drain(lo..));
                continue;
            }
            edges.push(Edge {
                from,
                to,
                len_km: chain.iter().map(|m| m.len_km).sum(),
                t0_min: chain.iter().map(|m| m.t0_min).sum(),
                toll: chain.iter().map(|m| m.toll).sum(),
                m_lo: lo as u32,
                m_hi: members.len() as u32,
            });
        }
        // Chains of interior nodes walked in a direction that is never usable
        // (e.g. both ends oneway inward) leave traversals nobody visited.
        let key = |m: &Member| (m.link, m.from);
        let visited: std::collections::HashSet<(u32, u32)> =
            members.iter().chain(&dead).map(key).collect();
        let unvisited: Vec<Member> = trav
            .iter()
            .filter(|m| !visited.contains(&key(m)))
            .cloned()
            .collect();
        dead.extend(unvisited);

        // CSR by `from`, keeping each edge's member range.
        let mut order: Vec<usize> = (0..edges.len()).collect();
        order.sort_by_key(|&e| edges[e].from);
        let edges: Vec<Edge> = order.into_iter().map(|e| edges[e].clone()).collect();
        let mut offsets = vec![0u32; n + 1];
        for e in &edges {
            offsets[e.from as usize + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let to = edges.iter().map(|e| e.to).collect();
        RoadGraph {
            n_nodes: n,
            index,
            offsets,
            edges,
            to,
            members,
            dead,
        }
    }

    pub fn free_flow_times(&self) -> Vec<f64> {
        self.edges.iter().map(|e| e.t0_min).collect()
    }

    /// Members of edge `e`.
    pub fn members_of(&self, e: usize) -> &[Member] {
        let ed = &self.edges[e];
        &self.members[ed.m_lo as usize..ed.m_hi as usize]
    }

    /// Identifies this contracted graph: a hash of every edge's endpoints and
    /// every member's free-flow time and capacity. Saved road volumes
    /// (`RoadState`) only apply to the graph they came from.
    /// FNV-1a, so the value is stable across Rust versions and saved files
    /// keep matching.
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |x: u64| {
            for b in x.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        };
        eat(self.n_nodes as u64);
        eat(self.edges.len() as u64);
        for e in 0..self.edges.len() {
            for m in self.members_of(e) {
                eat(((m.from as u64) << 32) | m.to as u64);
                eat(m.t0_min.to_bits());
                eat(m.cap_vph.to_bits());
            }
        }
        h
    }

    /// BPR time of edge `e` at `vol` vehicles/hour: the sum over its members.
    pub fn bpr_time(&self, e: usize, vol: f64, alpha: f64, beta: f64) -> f64 {
        self.members_of(e)
            .iter()
            .map(|m| m.t0_min * (1.0 + alpha * (vol / m.cap_vph).powf(beta)))
            .sum()
    }
}

/// Heap key: cost as f32 bits in the high word, node in the low word. For
/// non-negative floats the bit pattern orders like the value, so one u64
/// compare replaces a float compare plus tie-break (about 1.5x faster Dijkstra).
fn key(cost: f64, node: u32) -> Reverse<u64> {
    Reverse(((cost as f32).to_bits() as u64) << 32 | node as u64)
}

/// Reusable per-thread buffers for one shortest-path tree.
pub struct Tree {
    pub dist: Vec<f64>,
    pub pred: Vec<u32>,
    /// Nodes in the order they were settled.
    pub order: Vec<u32>,
    heap: BinaryHeap<Reverse<u64>>,
    settled: Vec<bool>,
    is_target: Vec<bool>,
    n_targets: usize,
    /// Stop expanding past this cost (F34 strategic cutoff); infinite by default.
    pub bound: f64,
    /// Nodes given a distance by the last run, so the next one resets only
    /// those (a bounded tree touches a sliver of a statewide graph).
    touched: Vec<u32>,
    target_list: Vec<u32>,
}

impl Tree {
    pub fn new(n: usize, targets: &[u32]) -> Tree {
        let mut is_target = vec![false; n];
        for &t in targets {
            is_target[t as usize] = true;
        }
        let target_list: Vec<u32> = (0..n as u32).filter(|&v| is_target[v as usize]).collect();
        let n_targets = target_list.len();
        Tree {
            dist: vec![f64::INFINITY; n],
            pred: vec![NONE; n],
            order: Vec::new(),
            heap: BinaryHeap::new(),
            settled: vec![false; n],
            is_target,
            n_targets,
            bound: f64::INFINITY,
            touched: Vec::new(),
            target_list,
        }
    }

    /// Dijkstra from `src` on edge costs `times`. Stops once every target is
    /// settled or the next node is costlier than `bound`.
    pub fn run(&mut self, g: &RoadGraph, times: &[f64], src: u32) {
        for &v in &self.touched {
            let v = v as usize;
            self.dist[v] = f64::INFINITY;
            self.pred[v] = NONE;
            self.settled[v] = false;
        }
        self.touched.clear();
        self.order.clear();
        self.heap.clear();
        let mut remaining = self.n_targets;
        self.dist[src as usize] = 0.0;
        self.touched.push(src);
        self.heap.push(key(0.0, src));
        while let Some(Reverse(k)) = self.heap.pop() {
            let v = (k & 0xFFFF_FFFF) as usize;
            if self.settled[v] {
                continue;
            }
            // Use the exact f64 distance, not the f32 key, so costs do not drift.
            let cost = self.dist[v];
            if cost > self.bound {
                break;
            }
            self.settled[v] = true;
            self.order.push(v as u32);
            if self.is_target[v] {
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
            let (lo, hi) = (g.offsets[v] as usize, g.offsets[v + 1] as usize);
            for ((ei, &to), &t) in (lo..hi).zip(&g.to[lo..hi]).zip(&times[lo..hi]) {
                let to = to as usize;
                let nd = cost + t;
                if nd < self.dist[to] && !self.settled[to] {
                    if self.dist[to] == f64::INFINITY {
                        self.touched.push(to as u32);
                    }
                    self.dist[to] = nd;
                    self.pred[to] = ei as u32;
                    self.heap.push(key(nd, to as u32));
                }
            }
        }
    }

    /// Replace the target set (the tree stops once all are settled).
    pub fn set_targets(&mut self, targets: impl IntoIterator<Item = u32>) {
        for &t in &self.target_list {
            self.is_target[t as usize] = false;
        }
        self.target_list.clear();
        for t in targets {
            if !self.is_target[t as usize] {
                self.is_target[t as usize] = true;
                self.target_list.push(t);
            }
        }
        self.n_targets = self.target_list.len();
    }

    /// True if `v` was settled (its `dist` is final). Nodes past `bound` are not.
    pub fn reached(&self, v: usize) -> bool {
        self.settled[v]
    }

    /// Distance and toll along the tree to every settled node.
    pub fn km_and_toll(&self, g: &RoadGraph, km: &mut [f64], toll: &mut [f64]) {
        for &v in &self.order {
            let p = self.pred[v as usize];
            if p == NONE {
                km[v as usize] = 0.0;
                toll[v as usize] = 0.0;
            } else {
                let e = &g.edges[p as usize];
                km[v as usize] = km[e.from as usize] + e.len_km;
                toll[v as usize] = toll[e.from as usize] + e.toll;
            }
        }
    }

    /// All-or-nothing loading: `node_demand` (destroyed) is pushed back up the
    /// tree and accumulated onto `edge_vol`.
    pub fn load(&self, g: &RoadGraph, node_demand: &mut [f64], edge_vol: &mut [f64]) {
        for &v in self.order.iter().rev() {
            let d = node_demand[v as usize];
            if d == 0.0 {
                continue;
            }
            let p = self.pred[v as usize];
            if p != NONE {
                edge_vol[p as usize] += d;
                node_demand[g.edges[p as usize].from as usize] += d;
            }
            node_demand[v as usize] = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;

    fn line_graph() -> ScenarioInput {
        let node = |id| RoadNode {
            id,
            lon: 0.0,
            lat: 0.0,
        };
        let link = |from, to, length_m, speed_kph| RoadLink {
            from,
            to,
            length_m,
            speed_kph,
            lanes: 1,
            capacity_vph: None,
            toll: 0.0,
            oneway: false,
        };
        ScenarioInput {
            road_nodes: vec![node(1), node(2), node(3)],
            // 1-2-3 slow local road vs direct 1-3 fast road.
            road_links: vec![
                link(1, 2, 1000.0, 30.0),
                link(2, 3, 1000.0, 30.0),
                link(1, 3, 3000.0, 120.0),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn dijkstra_prefers_faster_route_and_loads_it() {
        let g = RoadGraph::build_keeping(&line_graph(), &[1, 2, 3], true);
        let t = g.free_flow_times();
        let mut tree = Tree::new(g.n_nodes, &[0, 1, 2]);
        tree.run(&g, &t, 0);
        assert!((tree.dist[2] - 1.5).abs() < 1e-9); // 3 km at 120 kph
        assert!((tree.dist[1] - 2.0).abs() < 1e-9);
        let mut km = vec![0.0; 3];
        let mut toll = vec![0.0; 3];
        tree.km_and_toll(&g, &mut km, &mut toll);
        assert!((km[2] - 3.0).abs() < 1e-9);
        let mut dem = vec![0.0, 0.0, 10.0];
        let mut vol = vec![0.0; g.edges.len()];
        tree.load(&g, &mut dem, &mut vol);
        let loaded: Vec<_> = (0..g.edges.len()).filter(|&e| vol[e] > 0.0).collect();
        assert_eq!(loaded.len(), 1);
        assert_eq!(g.members_of(loaded[0])[0].link, 2);
    }

    /// Contraction must not change any zone-to-zone time, distance or toll.
    fn assert_same_zone_skims(input: &ScenarioInput, origins: usize) {
        let keep: Vec<u32> = input.zones.iter().map(|z| z.road_node).collect();
        let full = RoadGraph::build_keeping(input, &keep, false);
        let small = RoadGraph::build(input);
        assert!(small.edges.len() <= full.edges.len());
        let zn: Vec<u32> = keep.iter().map(|id| full.index[id]).collect();
        let (tf, ts) = (full.free_flow_times(), small.free_flow_times());
        let mut a = Tree::new(full.n_nodes, &zn);
        let mut b = Tree::new(small.n_nodes, &zn);
        let n = full.n_nodes;
        let (mut ka, mut ta, mut kb, mut tb) =
            (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for &o in zn.iter().step_by((zn.len() / origins).max(1)) {
            a.run(&full, &tf, o);
            b.run(&small, &ts, o);
            a.km_and_toll(&full, &mut ka, &mut ta);
            b.km_and_toll(&small, &mut kb, &mut tb);
            for &d in &zn {
                let d = d as usize;
                let (x, y) = (a.dist[d], b.dist[d]);
                assert!(
                    x == y || (x - y).abs() < 1e-9 * x.max(1.0),
                    "time {x} vs {y}"
                );
                if x.is_finite() {
                    // Ties between equal-time paths may pick different routes,
                    // so distance is compared only when unambiguous enough.
                    assert!((ka[d] - kb[d]).abs() < 1e-6 || (x - y).abs() < 1e-9);
                }
            }
        }
    }

    #[test]
    fn contraction_keeps_zone_times_on_synthetic_region() {
        let input = crate::synth::generate(&crate::synth::SynthConfig::small());
        assert_same_zone_skims(&input, 16);
    }

    /// Same check on a real pipeline build:
    /// `TRANSPO_SCENARIO=.../scenario.json cargo test -p sim-core --release -- --ignored`
    #[test]
    #[ignore]
    fn contraction_keeps_zone_times_on_real_scenario() {
        let path = std::env::var("TRANSPO_SCENARIO").expect("set TRANSPO_SCENARIO");
        let input = ScenarioInput::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let full = RoadGraph::build_keeping(&input, &[], false);
        let small = RoadGraph::build(&input);
        eprintln!(
            "edges {} -> {}, nodes with edges {} -> {}",
            full.edges.len(),
            small.edges.len(),
            full.offsets.windows(2).filter(|w| w[1] > w[0]).count(),
            small.offsets.windows(2).filter(|w| w[1] > w[0]).count()
        );
        assert_same_zone_skims(&input, 40);
    }
}
