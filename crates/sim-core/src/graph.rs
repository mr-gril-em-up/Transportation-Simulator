//! Road network as a compressed adjacency (CSR) graph, Dijkstra shortest-path
//! trees, car skims and all-or-nothing loading.

use crate::params::lane_capacity_vph;
use crate::schema::ScenarioInput;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

pub const NONE: u32 = u32::MAX;

/// One directed traversal of an input road link.
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    /// Index into `ScenarioInput::road_links`.
    pub link: u32,
    pub len_km: f64,
    pub t0_min: f64,
    pub cap_vph: f64,
    pub toll: f64,
}

pub struct RoadGraph {
    pub n_nodes: usize,
    pub index: HashMap<u32, u32>,
    /// `edges[offsets[v]..offsets[v+1]]` leave node `v`.
    pub offsets: Vec<u32>,
    pub edges: Vec<Edge>,
    /// `edges[e].to`, stored densely: the Dijkstra inner loop reads only this
    /// and the time array, so keeping it compact halves cache misses.
    pub to: Vec<u32>,
}

impl RoadGraph {
    pub fn build(input: &ScenarioInput) -> RoadGraph {
        let index: HashMap<u32, u32> = input
            .road_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i as u32))
            .collect();
        let mut edges = Vec::with_capacity(input.road_links.len() * 2);
        for (li, l) in input.road_links.iter().enumerate() {
            let (a, b) = (index[&l.from], index[&l.to]);
            let len_km = l.length_m / 1000.0;
            let cap = l
                .capacity_vph
                .unwrap_or(lane_capacity_vph(l.speed_kph) * l.lanes.max(1) as f64);
            let mk = |from, to| Edge {
                from,
                to,
                link: li as u32,
                len_km,
                t0_min: len_km / l.speed_kph * 60.0,
                cap_vph: cap.max(1.0),
                toll: l.toll,
            };
            edges.push(mk(a, b));
            if !l.oneway {
                edges.push(mk(b, a));
            }
        }
        edges.sort_by_key(|e| e.from);
        let n = input.road_nodes.len();
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
        }
    }

    pub fn free_flow_times(&self) -> Vec<f64> {
        self.edges.iter().map(|e| e.t0_min).collect()
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
}

impl Tree {
    pub fn new(n: usize, targets: &[u32]) -> Tree {
        let mut is_target = vec![false; n];
        for &t in targets {
            is_target[t as usize] = true;
        }
        let n_targets = is_target.iter().filter(|&&t| t).count();
        Tree {
            dist: vec![f64::INFINITY; n],
            pred: vec![NONE; n],
            order: Vec::new(),
            heap: BinaryHeap::new(),
            settled: vec![false; n],
            is_target,
            n_targets,
        }
    }

    /// Dijkstra from `src` on edge costs `times`. Stops once every target is settled.
    pub fn run(&mut self, g: &RoadGraph, times: &[f64], src: u32) {
        self.dist.fill(f64::INFINITY);
        self.pred.fill(NONE);
        self.settled.fill(false);
        self.order.clear();
        self.heap.clear();
        let mut remaining = self.n_targets;
        self.dist[src as usize] = 0.0;
        self.heap.push(key(0.0, src));
        while let Some(Reverse(k)) = self.heap.pop() {
            let v = (k & 0xFFFF_FFFF) as usize;
            if self.settled[v] {
                continue;
            }
            self.settled[v] = true;
            self.order.push(v as u32);
            if self.is_target[v] {
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
            // Use the exact f64 distance, not the f32 key, so costs do not drift.
            let cost = self.dist[v];
            let (lo, hi) = (g.offsets[v] as usize, g.offsets[v + 1] as usize);
            for ((ei, &to), &t) in (lo..hi).zip(&g.to[lo..hi]).zip(&times[lo..hi]) {
                let to = to as usize;
                let nd = cost + t;
                if nd < self.dist[to] && !self.settled[to] {
                    self.dist[to] = nd;
                    self.pred[to] = ei as u32;
                    self.heap.push(key(nd, to as u32));
                }
            }
        }
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
        let g = RoadGraph::build(&line_graph());
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
        let loaded: Vec<_> = g
            .edges
            .iter()
            .zip(&vol)
            .filter(|(_, v)| **v > 0.0)
            .collect();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0.link, 2);
    }
}
