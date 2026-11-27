//! Converts the pipeline's layers into sim-core's `ScenarioInput`.
//!
//! Ids are positions: zone id = index in zones.json, road node id = index in
//! roads.json `nodes`. The detail files and scenario.json therefore join
//! without a lookup table.

use crate::model::{Flow, RoadEdge, Roads, Zone};
use sim_core::{Commute, RoadLink, RoadNode, ScenarioInput};
use std::collections::HashMap;

/// Classes zones never load onto: a centroid connector to a freeway ramp
/// would let trips skip the local network.
fn is_access_class(class: &str) -> bool {
    !(class.starts_with("motorway") || class.starts_with("trunk"))
}

/// Lanes per direction from OSM's total-lane count.
fn lanes_per_direction(e: &RoadEdge) -> u8 {
    if e.oneway {
        e.lanes.max(1)
    } else {
        (e.lanes / 2).max(1)
    }
}

const CELL_DEG: f64 = 0.01; // about 1 km

/// Nearest access-class node to each point, by grid search. Falls back to the
/// nearest node of any class if no access node exists (tiny test graphs).
pub fn nearest_nodes(roads: &Roads, points: &[(f64, f64)]) -> Vec<Option<u32>> {
    let mut access = vec![false; roads.nodes.len()];
    for e in roads.edges.iter().filter(|e| is_access_class(&e.class)) {
        access[e.from as usize] = true;
        access[e.to as usize] = true;
    }
    let any_access = access.iter().any(|&a| a);
    let cell = |lon: f64, lat: f64| {
        (
            (lon / CELL_DEG).floor() as i32,
            (lat / CELL_DEG).floor() as i32,
        )
    };
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, n) in roads.nodes.iter().enumerate() {
        if access[i] || !any_access {
            grid.entry(cell(n[0], n[1])).or_default().push(i as u32);
        }
    }
    if grid.is_empty() {
        return vec![None; points.len()];
    }
    points
        .iter()
        .map(|&(lon, lat)| {
            let (cx, cy) = cell(lon, lat);
            let coslat = lat.to_radians().cos();
            let d2 = |i: u32| {
                let n = roads.nodes[i as usize];
                ((n[0] - lon) * coslat).powi(2) + (n[1] - lat).powi(2)
            };
            let mut best: Option<(f64, u32)> = None;
            // Expand rings until a hit, then one more ring (a closer node can sit in the next ring).
            for r in 0..=200 {
                for x in cx - r..=cx + r {
                    for y in cy - r..=cy + r {
                        if (x - cx).abs() != r && (y - cy).abs() != r {
                            continue;
                        }
                        for &i in grid.get(&(x, y)).into_iter().flatten() {
                            let d = d2(i);
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, i));
                            }
                        }
                    }
                }
                if let Some((d, _)) = best {
                    if d.sqrt() < r as f64 * CELL_DEG * coslat {
                        break;
                    }
                }
            }
            best.map(|b| b.1)
        })
        .collect()
}

/// Builds the scenario. Zones with no reachable road node (no roads file)
/// get road node 0 and a single placeholder node, so census-only builds still
/// validate; real builds always carry roads.
pub fn to_scenario(zones: &[Zone], flows: &[Flow], roads: Option<&Roads>) -> ScenarioInput {
    let empty = Roads {
        nodes: vec![],
        edges: vec![],
    };
    let roads = roads.unwrap_or(&empty);
    let pts: Vec<(f64, f64)> = zones.iter().map(|z| (z.lon, z.lat)).collect();
    let nearest = nearest_nodes(roads, &pts);
    let mut road_nodes: Vec<RoadNode> = roads
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| RoadNode {
            id: i as u32,
            lon: n[0],
            lat: n[1],
        })
        .collect();
    if road_nodes.is_empty() {
        road_nodes.push(RoadNode {
            id: 0,
            lon: pts[0].0,
            lat: pts[0].1,
        });
    }
    ScenarioInput {
        zones: zones
            .iter()
            .zip(&nearest)
            .enumerate()
            .map(|(i, (z, n))| sim_core::Zone {
                id: i as u32,
                lon: z.lon,
                lat: z.lat,
                population: z.population as f64,
                jobs: z.jobs as f64,
                road_node: n.unwrap_or(0),
                zero_car_share: 0.06,
                median_income: 70_000.0,
                parking_cost: 0.0,
                area_km2: None,
            })
            .collect(),
        road_nodes,
        road_links: roads
            .edges
            .iter()
            .filter(|e| e.length_m > 0.0 && e.from != e.to)
            .map(|e| RoadLink {
                from: e.from,
                to: e.to,
                length_m: e.length_m as f64,
                speed_kph: e.speed_kph as f64,
                lanes: lanes_per_direction(e),
                capacity_vph: None,
                toll: 0.0,
                oneway: e.oneway,
            })
            .collect(),
        commutes: flows
            .iter()
            .map(|f| Commute {
                home: f.home,
                work: f.work,
                workers: f.workers as f64,
            })
            .collect(),
        lines: vec![],
    }
}
