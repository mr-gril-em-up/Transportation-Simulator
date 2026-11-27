//! Coarser zone levels (F31): merge block groups into census tracts or
//! counties after the block-group build, so a statewide scenario has
//! thousands of zones instead of 18,638.
//!
//! GEOIDs nest, so a tract is the first 11 digits of a block group GEOID and
//! a county the first 5. Counts add up; the centroid, income and zero-car
//! share are population-weighted means of the block groups (DATA-5).

use crate::model::{Flow, Zone};
use anyhow::{bail, Result};
use std::collections::HashMap;

/// Zone level written to `zones.json` and `scenario.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    BlockGroup,
    Tract,
    County,
}

impl Level {
    /// Parse `bg`, `tract` or `county`.
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "bg" | "block_group" => Level::BlockGroup,
            "tract" => Level::Tract,
            "county" => Level::County,
            _ => bail!("unknown zone level {s:?} (bg, tract, county)"),
        })
    }

    /// Name written to the manifest.
    pub fn name(self) -> &'static str {
        match self {
            Level::BlockGroup => "block_group",
            Level::Tract => "tract",
            Level::County => "county",
        }
    }

    /// GEOID digits that identify a zone at this level.
    pub fn geoid_len(self) -> usize {
        match self {
            Level::BlockGroup => 12,
            Level::Tract => 11,
            Level::County => 5,
        }
    }
}

/// Merge block-group zones and flows into `level`. Zones keep the order in
/// which their first block group appears; flows between two block groups in
/// the same coarse zone become intrazonal flows, which sim-core accepts.
pub fn aggregate(zones: &[Zone], flows: &[Flow], level: Level) -> (Vec<Zone>, Vec<Flow>) {
    if level == Level::BlockGroup {
        return (zones.to_vec(), flows.to_vec());
    }
    let n = level.geoid_len();
    let mut index: HashMap<&str, u32> = HashMap::new();
    let mut groups: Vec<Vec<&Zone>> = Vec::new();
    let map: Vec<u32> = zones
        .iter()
        .map(|z| {
            let key = &z.geoid[..n.min(z.geoid.len())];
            *index.entry(key).or_insert_with(|| {
                groups.push(Vec::new());
                (groups.len() - 1) as u32
            })
        })
        .collect();
    for (z, &g) in zones.iter().zip(&map) {
        groups[g as usize].push(z);
    }
    let out_zones = groups.iter().map(|g| merge(g, n)).collect();

    let mut sums: HashMap<(u32, u32), u32> = HashMap::new();
    for f in flows {
        *sums
            .entry((map[f.home as usize], map[f.work as usize]))
            .or_default() += f.workers;
    }
    let mut out_flows: Vec<Flow> = sums
        .into_iter()
        .map(|((home, work), workers)| Flow {
            home,
            work,
            workers,
        })
        .collect();
    out_flows.sort_by_key(|f| (f.home, f.work));
    (out_zones, out_flows)
}

fn merge(g: &[&Zone], n: usize) -> Zone {
    let total = |f: fn(&Zone) -> u32| g.iter().map(|z| f(z)).sum::<u32>();
    let pop = total(|z| z.population);
    // Population weights; an unpopulated group falls back to equal weights.
    let w = |z: &Zone| if pop > 0 { z.population as f64 } else { 1.0 };
    let mean = |f: &dyn Fn(&Zone) -> Option<f64>| {
        let (s, ws) = g
            .iter()
            .filter_map(|z| f(z).map(|v| (v * w(z), w(z))))
            .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        (ws > 0.0).then(|| s / ws)
    };
    let area: Vec<f64> = g.iter().filter_map(|z| z.area_km2).collect();
    Zone {
        geoid: g[0].geoid[..n.min(g[0].geoid.len())].to_string(),
        lon: mean(&|z| Some(z.lon)).unwrap_or(g[0].lon),
        lat: mean(&|z| Some(z.lat)).unwrap_or(g[0].lat),
        population: pop,
        resident_workers: total(|z| z.resident_workers),
        jobs: total(|z| z.jobs),
        commute_out: total(|z| z.commute_out),
        commute_in: total(|z| z.commute_in),
        median_income: mean(&|z| z.median_income),
        zero_car_share: mean(&|z| z.zero_car_share),
        area_km2: (area.len() == g.len()).then(|| area.iter().sum()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(geoid: &str, lon: f64, pop: u32, income: Option<f64>, area: Option<f64>) -> Zone {
        Zone {
            geoid: geoid.into(),
            lon,
            lat: 30.0,
            population: pop,
            resident_workers: pop / 2,
            jobs: 10,
            commute_out: 1,
            commute_in: 2,
            median_income: income,
            zero_car_share: Some(0.1),
            area_km2: area,
        }
    }

    #[test]
    fn tracts_merge_counts_weights_and_flows() {
        let zones = vec![
            zone("484530011001", -97.0, 100, Some(50_000.0), Some(1.0)),
            zone("484530011002", -98.0, 300, Some(90_000.0), Some(2.0)),
            zone("484530012001", -99.0, 0, None, None),
        ];
        let flows = vec![
            Flow {
                home: 0,
                work: 1,
                workers: 5,
            },
            Flow {
                home: 1,
                work: 0,
                workers: 3,
            },
            Flow {
                home: 0,
                work: 2,
                workers: 7,
            },
        ];
        let (z, f) = aggregate(&zones, &flows, Level::Tract);
        assert_eq!(z.len(), 2);
        assert_eq!(z[0].geoid, "48453001100");
        assert_eq!(z[0].population, 400);
        assert_eq!(z[0].jobs, 20);
        assert!((z[0].lon + 97.75).abs() < 1e-9);
        assert_eq!(z[0].median_income, Some(80_000.0));
        assert_eq!(z[0].area_km2, Some(3.0));
        // Unpopulated tract: equal weights, no income, no area.
        assert_eq!(z[1].lon, -99.0);
        assert_eq!(z[1].median_income, None);
        assert_eq!(z[1].area_km2, None);
        assert_eq!(
            f,
            vec![
                Flow {
                    home: 0,
                    work: 0,
                    workers: 8
                },
                Flow {
                    home: 0,
                    work: 1,
                    workers: 7
                },
            ]
        );
        let (c, cf) = aggregate(&zones, &flows, Level::County);
        assert_eq!((c.len(), c[0].geoid.as_str(), cf.len()), (1, "48453", 1));
        assert_eq!(cf[0].workers, 15);
    }
}
