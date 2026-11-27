//! Intercity travel (F37, SIM-24): trips between places farther apart than the
//! strategic cutoff, which the tract-level tier drops.
//!
//! Zones are grouped into square cells (`IntercityParams::cell_km`). Every
//! pair of cells whose free-flow car time exceeds the cutoff gets a gravity
//! demand `scale x pop_o x pop_d / minutes^decay` (daily person trips, one
//! way), calibrated to the Dallas-Houston market (FRA Dallas-Houston HSR FEIS:
//! 15.97M person trips a year). Step 1 gives intercity trips one fixed car
//! share and loads the cars on the free-flow route as background road volume,
//! so rural interstates carry their through traffic. Mode choice with rail
//! and air is the next step.

use crate::graph::{RoadGraph, Tree};
use crate::params::IntercityParams;
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
    /// Daily person trips o -> d.
    pub trips: f64,
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
                        demand[v] += trips * p.car_share / p.car_occupancy;
                        out.push(IPair {
                            o,
                            d,
                            car_min: t as f32,
                            car_km: km[v] as f32,
                            trips,
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
}
