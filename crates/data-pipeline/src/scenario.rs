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

/// Marks nodes in the largest weakly connected component. Clipping at the
/// study-area margin leaves small road fragments; a zone loaded onto one
/// would validate but have no paths to anywhere.
pub fn largest_component(roads: &Roads) -> Vec<bool> {
    let mut parent: Vec<u32> = (0..roads.nodes.len() as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for e in &roads.edges {
        let (a, b) = (find(&mut parent, e.from), find(&mut parent, e.to));
        if a != b {
            parent[a as usize] = b;
        }
    }
    let roots: Vec<u32> = (0..parent.len() as u32)
        .map(|i| find(&mut parent, i))
        .collect();
    let mut size: HashMap<u32, usize> = HashMap::new();
    for &r in &roots {
        *size.entry(r).or_default() += 1;
    }
    let Some((&big, _)) = size
        .iter()
        .max_by_key(|(r, n)| (**n, std::cmp::Reverse(**r)))
    else {
        return vec![];
    };
    roots.iter().map(|&r| r == big).collect()
}

/// Nearest access-class node in the largest connected component to each
/// point, by grid search. Falls back to any node of that component if it has
/// no access-class node (tiny test graphs).
pub fn nearest_nodes(roads: &Roads, points: &[(f64, f64)]) -> Vec<Option<u32>> {
    let main = largest_component(roads);
    let mut access = vec![false; roads.nodes.len()];
    for e in roads.edges.iter().filter(|e| is_access_class(&e.class)) {
        access[e.from as usize] = main[e.from as usize];
        access[e.to as usize] = main[e.to as usize];
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
        if access[i] || (!any_access && main[i]) {
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

/// Shrinks the routable graph without changing any zone-to-zone path:
///
/// 1. Drops links outside the largest connected component.
/// 2. Prunes dead ends repeatedly: a node with one neighbor that is not a
///    zone connector can carry no zone-to-zone trip.
/// 3. Contracts pass-through nodes: a non-connector node with exactly two
///    links of equal speed, lanes and direction becomes one longer link.
///
/// Node ids are kept (sim-core allows sparse ids), so scenario links still
/// join roads.json by id. Returns the surviving node mask and links.
pub fn simplify(
    n_nodes: usize,
    links: Vec<RoadLink>,
    main: &[bool],
    protect: &[bool],
) -> (Vec<bool>, Vec<RoadLink>) {
    let mut links: Vec<Option<RoadLink>> = links
        .into_iter()
        .map(|l| (main[l.from as usize] && main[l.to as usize]).then_some(l))
        .collect();
    let mut inc: Vec<Vec<usize>> = vec![vec![]; n_nodes];
    for (i, l) in links.iter().enumerate() {
        if let Some(l) = l {
            inc[l.from as usize].push(i);
            inc[l.to as usize].push(i);
        }
    }
    let other = |l: &RoadLink, v: usize| if l.from as usize == v { l.to } else { l.from } as usize;

    // 2. Dead-end pruning.
    let mut queue: Vec<usize> = (0..n_nodes).collect();
    while let Some(v) = queue.pop() {
        if protect[v] || inc[v].is_empty() {
            continue;
        }
        let mut nbrs = inc[v].iter().map(|&i| other(links[i].as_ref().unwrap(), v));
        let first = nbrs.next().unwrap();
        if !nbrs.all(|u| u == first) || first == v {
            continue;
        }
        for i in std::mem::take(&mut inc[v]) {
            inc[first].retain(|&j| j != i);
            links[i] = None;
        }
        queue.push(first);
    }

    // 3. Degree-2 contraction. Merging v leaves its neighbors' link counts
    // unchanged, so one pass is enough.
    for v in 0..n_nodes {
        if protect[v] || inc[v].len() != 2 {
            continue;
        }
        let (i, j) = (inc[v][0], inc[v][1]);
        let (li, lj) = (links[i].clone().unwrap(), links[j].clone().unwrap());
        let (a, b) = (other(&li, v), other(&lj, v));
        if a == b || a == v || b == v {
            continue;
        }
        let same = li.speed_kph == lj.speed_kph && li.lanes == lj.lanes && li.oneway == lj.oneway;
        // One-way: must flow a -> v -> b (or b -> v -> a).
        let (from, to) = if !li.oneway || (li.to as usize == v && lj.from as usize == v) {
            (a, b)
        } else if lj.to as usize == v && li.from as usize == v {
            (b, a)
        } else {
            continue;
        };
        if !same {
            continue;
        }
        links[i] = Some(RoadLink {
            from: from as u32,
            to: to as u32,
            length_m: li.length_m + lj.length_m,
            ..li
        });
        links[j] = None;
        inc[v].clear();
        for x in &mut inc[b] {
            if *x == j {
                *x = i;
            }
        }
    }
    let keep: Vec<bool> = (0..n_nodes)
        .map(|v| !inc[v].is_empty() || protect[v])
        .collect();
    (keep, links.into_iter().flatten().collect())
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
    let links: Vec<RoadLink> = roads
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
            class: None,
        })
        .collect();
    let mut protect = vec![false; roads.nodes.len()];
    for n in nearest.iter().flatten() {
        protect[*n as usize] = true;
    }
    let (keep, road_links) = simplify(
        roads.nodes.len(),
        links,
        &largest_component(roads),
        &protect,
    );
    let mut road_nodes: Vec<RoadNode> = roads
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
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
                zero_car_share: z.zero_car_share.unwrap_or(0.06),
                median_income: z.median_income.unwrap_or(70_000.0),
                parking_cost: 0.0,
                area_km2: z.area_km2,
            })
            .collect(),
        road_nodes,
        road_links,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn link(from: u32, to: u32, speed: f64, oneway: bool) -> RoadLink {
        RoadLink {
            from,
            to,
            length_m: 100.0,
            speed_kph: speed,
            lanes: 1,
            capacity_vph: None,
            toll: 0.0,
            oneway,
            class: None,
        }
    }

    #[test]
    fn simplify_contracts_chains_and_prunes_dead_ends() {
        // Zones at 0 and 4. 0-1-2-3-4 is a two-way chain whose 2-3 link is faster.
        // 5 is a dead-end stub off 1. 6 -> 7 -> 8 is a one-way detour 0 -> 4.
        // 9 and 10 are a fragment outside the main component.
        let links = vec![
            link(0, 1, 50.0, false),
            link(1, 2, 50.0, false),
            link(2, 3, 80.0, false),
            link(3, 4, 80.0, false),
            link(1, 5, 50.0, false),
            link(0, 6, 50.0, true),
            link(6, 7, 50.0, true),
            link(8, 7, 50.0, true), // wrong way: 7 cannot be contracted
            link(8, 4, 50.0, true),
            link(9, 10, 50.0, false),
        ];
        let mut main = vec![true; 11];
        main[9] = false;
        main[10] = false;
        let mut protect = vec![false; 11];
        protect[0] = true;
        protect[4] = true;
        let (keep, out) = simplify(11, links, &main, &protect);
        let kept: Vec<usize> = (0..11).filter(|&i| keep[i]).collect();
        // 5 pruned; 1 and 3 contracted; 2 kept (speed changes); 6 contracted into
        // 0 -> 7; 7 (two in-links) and 8 (two out-links) are not pass-through.
        assert_eq!(kept, [0, 2, 4, 7, 8]);
        let has = |f, t, len: f64| {
            out.iter()
                .any(|l| l.from == f && l.to == t && (l.length_m - len).abs() < 1e-9)
        };
        assert!(has(0, 2, 200.0), "{out:?}");
        assert!(has(2, 4, 200.0), "{out:?}");
        assert!(has(0, 7, 200.0), "{out:?}");
        assert_eq!(out.len(), 5, "{out:?}");
    }
}
