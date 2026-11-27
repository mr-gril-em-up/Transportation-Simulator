//! Synthetic Travis-County-like scenario for tests, benchmarks and app
//! development before the real data pipeline lands. Deterministic per seed.
//! Not calibrated to real Austin; it only has the right shape and size.

use crate::geo::{dist_km, offset};
use crate::schema::*;

/// Size and shape of the synthetic region.
#[derive(Debug, Clone)]
pub struct SynthConfig {
    /// Random seed; same seed, same scenario.
    pub seed: u64,
    /// Square region side, km (Travis County is ~2,600 km², ~51 km square).
    pub extent_km: f64,
    /// Road grid spacing, km. 0.5 gives ~10k nodes, 0.25 gives ~40k.
    pub road_spacing_km: f64,
    /// Zones per side (26 gives 676, close to Travis County's block group count).
    pub zones_per_side: usize,
    /// Total residents.
    pub population: f64,
    /// Total jobs.
    pub jobs: f64,
}

impl Default for SynthConfig {
    fn default() -> Self {
        SynthConfig {
            seed: 7,
            extent_km: 50.0,
            road_spacing_km: 0.5,
            zones_per_side: 26,
            population: 1_300_000.0,
            jobs: 850_000.0,
        }
    }
}

impl SynthConfig {
    /// A small region that runs in milliseconds in debug builds (for unit tests).
    pub fn small() -> Self {
        SynthConfig {
            extent_km: 16.0,
            road_spacing_km: 1.0,
            zones_per_side: 8,
            population: 120_000.0,
            jobs: 80_000.0,
            ..Default::default()
        }
    }
}

/// Austin's downtown, the region's center.
pub const CENTER: (f64, f64) = (-97.7431, 30.2672);

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Generate the scenario. Layout: a road grid with arterials every 3 km and three
/// freeways (a north-south one through the center like I-35, one 5 km west like
/// MoPac, an east-west one 8 km north like US-183); population peaks near the
/// center, jobs peak downtown with a second center 14 km north (like the Domain).
pub fn generate(cfg: &SynthConfig) -> ScenarioInput {
    let mut rng = Rng(cfg.seed.wrapping_mul(2654435761).max(1));
    let half = cfg.extent_km / 2.0;
    let n = (cfg.extent_km / cfg.road_spacing_km).round() as usize + 1;
    let s = cfg.road_spacing_km;
    let xy = |ix: usize| -half + ix as f64 * s;

    let mut road_nodes = Vec::with_capacity(n * n);
    for iy in 0..n {
        for ix in 0..n {
            let (lon, lat) = offset(CENTER.0, CENTER.1, xy(ix), xy(iy));
            road_nodes.push(RoadNode {
                id: (iy * n + ix) as u32,
                lon,
                lat,
            });
        }
    }
    let arterial_every = ((3.0 / s).round() as usize).max(1);
    let near = |v: f64, target: f64| (v - target).abs() < s * 0.5;
    let class = |vertical: bool, fixed: usize| -> (f64, u8) {
        let v = xy(fixed);
        let freeway = if vertical {
            near(v, 0.0) || near(v, -5.0)
        } else {
            near(v, 8.0)
        };
        if freeway {
            (105.0, 3)
        } else if fixed.is_multiple_of(arterial_every) {
            (60.0, 2)
        } else {
            (40.0, 1)
        }
    };
    let mut road_links = Vec::with_capacity(2 * n * n);
    for iy in 0..n {
        for ix in 0..n {
            let id = (iy * n + ix) as u32;
            if ix + 1 < n {
                let (speed_kph, lanes) = class(false, iy);
                road_links.push(RoadLink {
                    from: id,
                    to: id + 1,
                    length_m: s * 1000.0,
                    speed_kph,
                    lanes,
                    capacity_vph: None,
                    toll: 0.0,
                    oneway: false,
                });
            }
            if iy + 1 < n {
                let (speed_kph, lanes) = class(true, ix);
                road_links.push(RoadLink {
                    from: id,
                    to: id + n as u32,
                    length_m: s * 1000.0,
                    speed_kph,
                    lanes,
                    capacity_vph: None,
                    toll: 0.0,
                    oneway: false,
                });
            }
        }
    }

    let zn = cfg.zones_per_side;
    let cell = cfg.extent_km / zn as f64;
    let mut zones = Vec::with_capacity(zn * zn);
    let (mut pw, mut jw) = (Vec::new(), Vec::new());
    for zy in 0..zn {
        for zx in 0..zn {
            let x = -half + (zx as f64 + 0.2 + 0.6 * rng.next()) * cell;
            let y = -half + (zy as f64 + 0.2 + 0.6 * rng.next()) * cell;
            let r = (x * x + y * y).sqrt();
            let domain = ((x - 2.0).powi(2) + (y - 14.0).powi(2)).sqrt();
            pw.push((-r / 12.0).exp() * (0.6 + 0.8 * rng.next()));
            jw.push(
                ((-r / 3.0).exp() * 3.0 + (-domain / 2.0).exp() * 1.2 + (-r / 15.0).exp() * 0.4)
                    * (0.5 + rng.next()),
            );
            let (lon, lat) = offset(CENTER.0, CENTER.1, x, y);
            let ix = (((x + half) / s).round() as usize).min(n - 1);
            let iy = (((y + half) / s).round() as usize).min(n - 1);
            zones.push(Zone {
                id: (zy * zn + zx) as u32 + 1,
                lon,
                lat,
                population: 0.0,
                jobs: 0.0,
                road_node: (iy * n + ix) as u32,
                zero_car_share: (0.03 + 0.15 * (-r / 4.0).exp()).min(0.25),
                median_income: 45_000.0 + 75_000.0 * rng.next(),
                parking_cost: if r < 1.5 {
                    15.0
                } else if r < 3.0 {
                    6.0
                } else {
                    0.0
                },
                area_km2: Some(cell * cell),
            });
        }
    }
    let (ps, js): (f64, f64) = (pw.iter().sum(), jw.iter().sum());
    for (i, z) in zones.iter_mut().enumerate() {
        z.population = (cfg.population * pw[i] / ps).round();
        z.jobs = (cfg.jobs * jw[i] / js).round();
    }

    // Commutes: workers (~52% of residents) spread over jobs with distance decay.
    let mut commutes = Vec::new();
    for h in &zones {
        let workers = h.population * 0.52;
        let w: Vec<f64> = zones
            .iter()
            .map(|j| j.jobs * (-dist_km(h.lon, h.lat, j.lon, j.lat) / 9.0).exp())
            .collect();
        let sum: f64 = w.iter().sum();
        for (k, j) in zones.iter().enumerate() {
            let f = workers * w[k] / sum;
            if f >= 1.0 {
                commutes.push(Commute {
                    home: h.id,
                    work: j.id,
                    workers: f.round(),
                });
            }
        }
    }

    ScenarioInput {
        zones,
        road_nodes,
        road_links,
        commutes,
        lines: Vec::new(),
    }
}

/// A straight line from (x0,y0) to (x1,y1) km relative to `CENTER` with stops
/// every `spacing_km`. Handy for tests and the benchmark.
pub fn straight_line(
    mode: Mode,
    from_km: (f64, f64),
    to_km: (f64, f64),
    spacing_km: f64,
    headway_min: f64,
) -> TransitLine {
    let len = ((to_km.0 - from_km.0).powi(2) + (to_km.1 - from_km.1).powi(2)).sqrt();
    let k = (len / spacing_km).round().max(1.0) as usize;
    let stops = (0..=k)
        .map(|i| {
            let t = i as f64 / k as f64;
            let (lon, lat) = offset(
                CENTER.0,
                CENTER.1,
                from_km.0 + t * (to_km.0 - from_km.0),
                from_km.1 + t * (to_km.1 - from_km.1),
            );
            Stop {
                lon,
                lat,
                name: format!("Stop {i}"),
                park_ride: i == 0 || i == k,
            }
        })
        .collect();
    TransitLine {
        name: format!("{mode:?}"),
        mode,
        stops,
        headway_min,
        offpeak_headway_min: None,
        fare: None,
        fare_per_km: None,
        speed_kph: None,
        span_hours: None,
    }
}
