//! Intercity travel (F37, SIM-24): trips between places farther apart than the
//! strategic cutoff, which the tract-level tier drops.
//!
//! Zones are grouped into square cells (`IntercityParams::cell_km`). Every
//! pair of cells whose free-flow car time exceeds the cutoff gets a gravity
//! demand `scale x pop_o x pop_d / minutes^decay` (daily person trips, one
//! way), calibrated to the Dallas-Houston market (FRA Dallas-Houston HSR FEIS:
//! 15.97M person trips a year).
//!
//! Mode choice (step 2, SIM-25) is a logit over car, air and rail
//! (`IntercityParams::shares`). Air flies between Texas commercial airports
//! (`AIRPORTS`); rail is any one line with a stop within driving range of
//! both cells, no transfers. Cars at the base car share (car against air,
//! no rail) load the free-flow route as background road volume, so rural
//! interstates carry their through traffic. That volume is fixed at start:
//! riders a new rail line wins do not leave the roads.

use crate::geo::{dist_km, PointGrid};
use crate::graph::{RoadGraph, Tree};
use crate::params::IntercityParams;
use crate::transit::{LineLoads, TransitNet};
use rayon::prelude::*;
use std::collections::HashMap;

/// A group of zones treated as one place for intercity travel.
pub struct Cell {
    /// Population-weighted centroid.
    pub lon: f64,
    pub lat: f64,
    /// Residents.
    pub pop: f64,
    /// Graph node of the cell's most populous zone.
    pub node: u32,
}

/// One direction of an intercity cell pair.
pub struct IPair {
    pub o: u32,
    pub d: u32,
    pub car_min: f32,
    pub car_km: f32,
    /// Daily person trips o -> d, all modes.
    pub trips: f64,
    /// Car generalized minutes: time plus running cost and tolls per person.
    pub car_gc: f32,
    /// Best air generalized minutes, without the constant (infinite: no flight).
    pub air_gc: f32,
}

/// Texas commercial airports: code, lon, lat, hub. A flight needs a hub at
/// one end or both (regional airports fly to the hubs, not to each other).
pub const AIRPORTS: [(&str, f64, f64, bool); 24] = [
    ("DFW", -97.038, 32.897, true),
    ("DAL", -96.851, 32.847, true),
    ("IAH", -95.341, 29.984, true),
    ("HOU", -95.279, 29.645, true),
    ("AUS", -97.670, 30.194, false),
    ("SAT", -98.469, 29.534, false),
    ("ELP", -106.377, 31.807, false),
    ("MAF", -102.202, 31.943, false),
    ("LBB", -101.823, 33.664, false),
    ("AMA", -101.706, 35.219, false),
    ("CRP", -97.501, 27.770, false),
    ("MFE", -98.239, 26.176, false),
    ("HRL", -97.654, 26.229, false),
    ("BRO", -97.426, 25.907, false),
    ("TYR", -95.402, 32.354, false),
    ("ABI", -99.682, 32.411, false),
    ("SJT", -100.496, 31.358, false),
    ("LRD", -99.462, 27.544, false),
    ("BPT", -94.021, 29.951, false),
    ("CLL", -96.364, 30.589, false),
    ("ACT", -97.230, 31.611, false),
    ("GRK", -97.828, 31.067, false),
    ("SPS", -98.491, 33.989, false),
    ("GGG", -94.711, 32.384, false),
];

/// The best one-line rail trip for a pair.
#[derive(Clone, Copy)]
pub struct RailOpt {
    /// Generalized minutes, without the constant.
    pub gc: f64,
    pub line: u32,
    /// Boarding and alighting stop indexes on the line.
    pub a: u32,
    pub b: u32,
    pub km: f64,
}

/// Intercity mode choice against the current transit network.
pub struct Choice {
    /// Per pair (aligned to `Intercity::pairs`), the best rail trip.
    pub rail: Vec<Option<RailOpt>>,
    /// Daily person trips by rail and by air.
    pub rail_trips: f64,
    pub air_trips: f64,
    /// Intercity riders on each line.
    pub loads: LineLoads,
}

pub struct Intercity {
    pub cells: Vec<Cell>,
    pub pairs: Vec<IPair>,
    /// Daily intercity car trips (vehicles) per graph edge.
    pub car_veh_daily: Vec<f64>,
}

impl Intercity {
    /// `zones` are (lon, lat, population) in zone order, `zone_node` their
    /// graph nodes, `times` free-flow edge times, `cut` the strategic cutoff
    /// in minutes.
    pub fn build(
        zones: &[(f64, f64, f64)],
        zone_node: &[u32],
        g: &RoadGraph,
        times: &[f64],
        p: &IntercityParams,
        cut: f64,
        car_cost_per_km: f64,
    ) -> Intercity {
        // Square cells on a local equirectangular projection at Texas's mid latitude.
        let km_lon = 111.32 * 31f64.to_radians().cos();
        let key = |lon: f64, lat: f64| {
            (
                (lon * km_lon / p.cell_km).floor() as i64,
                (lat * 111.32 / p.cell_km).floor() as i64,
            )
        };
        let mut index: HashMap<(i64, i64), u32> = HashMap::new();
        let mut cells: Vec<Cell> = Vec::new();
        let mut biggest: Vec<f64> = Vec::new();
        for (zi, &(lon, lat, pop)) in zones.iter().enumerate() {
            let c = *index.entry(key(lon, lat)).or_insert_with(|| {
                cells.push(Cell {
                    lon: 0.0,
                    lat: 0.0,
                    pop: 0.0,
                    node: zone_node[zi],
                });
                biggest.push(-1.0);
                cells.len() as u32 - 1
            });
            let cell = &mut cells[c as usize];
            cell.lon += lon * pop;
            cell.lat += lat * pop;
            cell.pop += pop;
            if pop > biggest[c as usize] {
                biggest[c as usize] = pop;
                cell.node = zone_node[zi];
            }
        }
        for c in &mut cells {
            if c.pop > 0.0 {
                c.lon /= c.pop;
                c.lat /= c.pop;
            }
        }
        let live: Vec<u32> = (0..cells.len() as u32)
            .filter(|&c| cells[c as usize].pop > 0.0)
            .collect();
        let targets: Vec<u32> = live.iter().map(|&c| cells[c as usize].node).collect();
        let vot = p.vot_per_hour / 60.0;
        // Per cell: airports in range and the drive to each, minutes.
        let ports: Vec<Vec<(usize, f64)>> = cells
            .iter()
            .map(|c| {
                AIRPORTS
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| {
                        let km = dist_km(c.lon, c.lat, a.1, a.2);
                        (km <= p.max_access_km).then(|| (i, km / p.access_kph * 60.0))
                    })
                    .collect()
            })
            .collect();
        let air_gc = |o: usize, d: usize| -> f64 {
            let mut best = f64::INFINITY;
            for &(i, ao) in &ports[o] {
                for &(j, ad) in &ports[d] {
                    let (a, b) = (&AIRPORTS[i], &AIRPORTS[j]);
                    let km = dist_km(a.1, a.2, b.1, b.2);
                    if i == j || km < p.min_air_km || !(a.3 || b.3) {
                        continue;
                    }
                    let min = ao + ad + p.air_terminal_min + p.air_taxi_min + km / p.air_kph * 60.0;
                    let fare = p.air_fare_base + p.air_fare_per_km * km;
                    best = best.min(min + fare / vot);
                }
            }
            best
        };
        let n_edges = g.edges.len();
        let (pairs, car_veh_daily) = live
            .par_iter()
            .map_init(
                || (Tree::new(g.n_nodes, &targets), vec![0.0; g.n_nodes]),
                |(tree, demand), &o| {
                    let co = &cells[o as usize];
                    demand.iter_mut().for_each(|x| *x = 0.0);
                    tree.run(g, times, co.node);
                    let mut km = vec![0.0; g.n_nodes];
                    let mut toll = vec![0.0; g.n_nodes];
                    tree.km_and_toll(g, &mut km, &mut toll);
                    let mut out = Vec::new();
                    let mut vol = vec![0.0; n_edges];
                    for &d in &live {
                        let cd = &cells[d as usize];
                        let v = cd.node as usize;
                        if d == o || !tree.reached(v) || tree.dist[v] <= cut {
                            continue;
                        }
                        let t = tree.dist[v];
                        let trips = p.scale * co.pop * cd.pop / t.powf(p.decay);
                        let car_gc =
                            t + (km[v] * car_cost_per_km + toll[v]) / p.car_occupancy / vot;
                        let air = air_gc(o as usize, d as usize);
                        let share = p.shares(car_gc, air, f64::INFINITY)[0];
                        demand[v] += trips * share / p.car_occupancy;
                        out.push(IPair {
                            o,
                            d,
                            car_min: t as f32,
                            car_km: km[v] as f32,
                            trips,
                            car_gc: car_gc as f32,
                            air_gc: air as f32,
                        });
                    }
                    tree.load(g, demand, &mut vol);
                    (out, vol)
                },
            )
            .reduce(
                || (Vec::new(), vec![0.0; n_edges]),
                |(mut pa, mut va), (pb, vb)| {
                    pa.extend(pb);
                    va.iter_mut().zip(&vb).for_each(|(a, b)| *a += b);
                    (pa, va)
                },
            );
        let mut pairs = pairs;
        pairs.sort_by_key(|q| (q.o, q.d));
        Intercity {
            cells,
            pairs,
            car_veh_daily,
        }
    }

    /// Daily intercity person trips, all modes.
    pub fn person_trips(&self) -> f64 {
        self.pairs.iter().map(|q| q.trips).sum()
    }

    /// Best one-line rail trip per pair on `net`: drive to the nearest stop
    /// within `max_access_km` of each cell, wait half the off-peak headway
    /// (capped), ride, pay the line's fare.
    pub fn rail_options(&self, net: &TransitNet, p: &IntercityParams) -> Vec<Option<RailOpt>> {
        let vot = p.vot_per_hour / 60.0;
        let mut best: Vec<Option<RailOpt>> = vec![None; self.pairs.len()];
        let grid = PointGrid::new(self.cells.iter().map(|c| (c.lon, c.lat)), p.max_access_km);
        let mut near = Vec::new();
        for (li, l) in net.lines.iter().enumerate() {
            // Per cell, the nearest stop of this line in range: (cell, stop, drive min).
            let mut served: HashMap<u32, (u32, f64)> = HashMap::new();
            for s in 0..=l.seg_km.len() {
                let pl = &net.platforms[l.first_platform as usize + s];
                grid.near(pl.lon, pl.lat, p.max_access_km, &mut near);
                for &c in &near {
                    let cell = &self.cells[c as usize];
                    let km = dist_km(cell.lon, cell.lat, pl.lon, pl.lat);
                    if cell.pop <= 0.0 || km > p.max_access_km {
                        continue;
                    }
                    let min = km / p.access_kph * 60.0;
                    let e = served.entry(c).or_insert((s as u32, min));
                    if min < e.1 {
                        *e = (s as u32, min);
                    }
                }
            }
            if served.len() < 2 {
                continue;
            }
            let mut served: Vec<(u32, u32, f64)> =
                served.into_iter().map(|(c, (s, m))| (c, s, m)).collect();
            served.sort_by_key(|x| x.0);
            let wait = (l.offpeak_headway / 2.0).min(p.max_wait_min);
            for &(o, a, ao) in &served {
                for &(d, b, ad) in &served {
                    if a == b {
                        continue;
                    }
                    let Ok(k) = self.pairs.binary_search_by_key(&(o, d), |q| (q.o, q.d)) else {
                        continue;
                    };
                    let (lo, hi) = (a.min(b) as usize, a.max(b) as usize);
                    let ride: f64 = l.seg_min[lo..hi].iter().sum();
                    let km: f64 = l.seg_km[lo..hi].iter().sum();
                    let fare = l.fare + l.fare_per_km * km;
                    let gc = ao + ad + p.station_min + wait + ride + fare / vot;
                    if best[k].is_none_or(|r| gc < r.gc) {
                        best[k] = Some(RailOpt {
                            gc,
                            line: li as u32,
                            a,
                            b,
                            km,
                        });
                    }
                }
            }
        }
        best
    }

    /// Mode choice on `net`, with the rail riders loaded on their lines.
    pub fn choose(&self, net: &TransitNet, p: &IntercityParams) -> Choice {
        let rail = self.rail_options(net, p);
        let mut loads = LineLoads::new(net);
        let (mut rail_trips, mut air_trips) = (0.0, 0.0);
        for (q, r) in self.pairs.iter().zip(&rail) {
            let s = p.shares(
                q.car_gc as f64,
                q.air_gc as f64,
                r.map_or(f64::INFINITY, |r| r.gc),
            );
            air_trips += q.trips * s[1];
            let Some(r) = r else { continue };
            let t = q.trips * s[2];
            rail_trips += t;
            let li = r.line as usize;
            loads.boardings[li] += t;
            loads.stop_boardings[li][r.a as usize] += t;
            loads.pkm[li] += t * r.km;
            let dir = usize::from(r.b < r.a);
            for seg in r.a.min(r.b)..r.a.max(r.b) {
                loads.seg[li][seg as usize][dir] += t;
            }
        }
        Choice {
            rail,
            rail_trips,
            air_trips,
            loads,
        }
    }
}
