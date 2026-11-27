//! Road routing for road-running transit (bus, trolleybus, BRT): snap stops to
//! the road graph and find the shortest free-flow distance between them, so a
//! drawn bus line's length follows the streets instead of a straight line
//! times `alignment_factor` (SIM-8). Also returns the street geometry so the
//! app can draw the route.
//!
//! Routes are cached per (from node, to node), so re-evaluating all lines on
//! every edit only routes new stop pairs.

use crate::geo::dist_km;
use crate::graph::RoadGraph;
use crate::schema::{Mode, ScenarioInput};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

/// Farthest a stop may be from a road node to snap to it, km.
pub const SNAP_KM: f64 = 0.5;
/// A route longer than this times the straight-line distance is treated as a
/// bad snap (a one-way loop or the wrong side of a river) and falls back to
/// `alignment_factor`. Real stop-to-stop bus circuity is ~1.2-1.5.
pub const MAX_DETOUR: f64 = 2.5;
/// Grid cell for the snap index, degrees (~1 km).
const CELL_DEG: f64 = 0.01;

/// A routed stop-to-stop segment.
#[derive(Debug, Clone)]
pub struct Route {
    /// Road distance plus the walk-free hops from each stop to its snapped node, km.
    pub km: f64,
    /// [lon, lat] points from stop a, through road nodes, to stop b.
    pub shape: Vec<[f64; 2]>,
}

/// True for modes that run on the street network.
pub fn runs_on_roads(m: Mode) -> bool {
    matches!(m, Mode::Bus | Mode::Trolleybus | Mode::Brt)
}

/// Snap index and route cache over the contracted road graph.
#[derive(Debug, Default)]
pub struct Router {
    coords: Vec<[f64; 2]>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    cache: HashMap<(u32, u32), Option<Arc<Route>>>,
}

fn cell(lon: f64, lat: f64) -> (i32, i32) {
    (
        (lon / CELL_DEG).floor() as i32,
        (lat / CELL_DEG).floor() as i32,
    )
}

impl Router {
    /// Index the graph's routable nodes (those with an edge out, i.e. not
    /// pruned and not contracted away).
    pub fn new(input: &ScenarioInput, g: &RoadGraph) -> Router {
        let coords: Vec<[f64; 2]> = input.road_nodes.iter().map(|n| [n.lon, n.lat]).collect();
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (v, &[lon, lat]) in coords.iter().enumerate().take(g.n_nodes) {
            if g.offsets[v + 1] > g.offsets[v] {
                grid.entry(cell(lon, lat)).or_default().push(v as u32);
            }
        }
        Router {
            coords,
            grid,
            cache: HashMap::new(),
        }
    }

    /// Nearest routable node within `SNAP_KM`. Searches the 3x3 cells around
    /// the point (cells are ~1 km, so this covers 0.5 km); ties go to the
    /// lower node index, keeping results deterministic.
    pub fn snap(&self, lon: f64, lat: f64) -> Option<u32> {
        let (cx, cy) = cell(lon, lat);
        let mut best: Option<(f64, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &v in self.grid.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let [x, y] = self.coords[v as usize];
                    let d = dist_km(lon, lat, x, y);
                    if d <= SNAP_KM && best.is_none_or(|(bd, bv)| (d, v) < (bd, bv)) {
                        best = Some((d, v));
                    }
                }
            }
        }
        best.map(|(_, v)| v)
    }

    /// Route between two stops, or None if either is off the road network, no
    /// path exists, or the route is over `MAX_DETOUR` times the straight line
    /// (the caller falls back to `alignment_factor`).
    pub fn route(&mut self, g: &RoadGraph, a: [f64; 2], b: [f64; 2]) -> Option<Arc<Route>> {
        let (na, nb) = (self.snap(a[0], a[1])?, self.snap(b[0], b[1])?);
        let core = match self.cache.get(&(na, nb)) {
            Some(r) => r.clone(),
            None => {
                let r = self.shortest(g, na, nb).map(Arc::new);
                self.cache.insert((na, nb), r.clone());
                r
            }
        }?;
        // Add the stop-to-node hops at each end.
        let [ax, ay] = self.coords[na as usize];
        let [bx, by] = self.coords[nb as usize];
        let km = core.km + dist_km(a[0], a[1], ax, ay) + dist_km(b[0], b[1], bx, by);
        if km > MAX_DETOUR * dist_km(a[0], a[1], b[0], b[1]).max(0.05) {
            return None;
        }
        let mut shape = Vec::with_capacity(core.shape.len() + 2);
        shape.push(a);
        shape.extend_from_slice(&core.shape);
        shape.push(b);
        Some(Arc::new(Route { km, shape }))
    }

    /// A* by distance from node `s` to node `t` (straight-line heuristic).
    /// Returns road km and node coordinates along the expanded path.
    fn shortest(&self, g: &RoadGraph, s: u32, t: u32) -> Option<Route> {
        let [tx, ty] = self.coords[t as usize];
        let h = |v: u32| {
            let [x, y] = self.coords[v as usize];
            dist_km(x, y, tx, ty)
        };
        let n = g.n_nodes;
        let mut dist = vec![f64::INFINITY; n];
        let mut via = vec![u32::MAX; n]; // edge index used to reach the node
        let mut heap = BinaryHeap::new();
        dist[s as usize] = 0.0;
        // Keys are f64 bits: non-negative floats order like their bit patterns.
        heap.push(Reverse((h(s).to_bits(), s)));
        while let Some(Reverse((key, v))) = heap.pop() {
            if v == t {
                break;
            }
            let dv = dist[v as usize];
            if key > (dv + h(v)).to_bits() {
                continue; // stale entry
            }
            let (lo, hi) = (g.offsets[v as usize], g.offsets[v as usize + 1]);
            for e in lo..hi {
                let w = g.to[e as usize];
                let nd = dv + g.edges[e as usize].len_km;
                if nd < dist[w as usize] {
                    dist[w as usize] = nd;
                    via[w as usize] = e;
                    heap.push(Reverse(((nd + h(w)).to_bits(), w)));
                }
            }
        }
        if !dist[t as usize].is_finite() {
            return None;
        }
        // Walk back along edges, then expand each edge's member traversals.
        let mut path = Vec::new();
        let mut v = t;
        while v != s {
            let e = via[v as usize];
            path.push(e);
            v = g.edges[e as usize].from;
        }
        path.reverse();
        let mut shape = vec![self.coords[s as usize]];
        for e in path {
            let ed = &g.edges[e as usize];
            for m in &g.members[ed.m_lo as usize..ed.m_hi as usize] {
                shape.push(self.coords[m.to as usize]);
            }
        }
        Some(Route {
            km: dist[t as usize],
            shape,
        })
    }
}
