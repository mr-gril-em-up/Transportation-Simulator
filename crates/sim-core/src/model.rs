//! The `Sim`: owns the scenario, runs demand -> mode choice -> assignment, and
//! answers the app's questions. See docs/MODEL.md for the math and the reasons
//! behind each simplification.

use crate::clock::Clock;
use crate::geo::dist_km;
use crate::graph::{RoadGraph, Tree};
use crate::params::*;
use crate::schema::*;
use crate::transit::{Access, LineLoads, Searcher, TOd, TransitNet, ZonePt};
use crate::SimError;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Stable handle for a transit line. Never reused within a `Sim`.
pub type LineId = u32;

/// Trips by choice, in the same order as the `DRIVE..BIKE` constants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ModeSplit {
    /// Drive alone.
    pub drive: f64,
    /// Shared ride (driver or passenger).
    pub carpool: f64,
    /// Transit reached on foot.
    pub transit_walk: f64,
    /// Transit reached by car (park-and-ride).
    pub transit_drive: f64,
    /// Walk the whole trip.
    pub walk: f64,
    /// Bike the whole trip.
    pub bike: f64,
}

impl ModeSplit {
    /// Build from an array indexed by the `params::DRIVE..BIKE` constants.
    pub fn from_array(a: [f64; N_CHOICES]) -> Self {
        ModeSplit {
            drive: a[DRIVE],
            carpool: a[CARPOOL],
            transit_walk: a[TRANSIT_WALK],
            transit_drive: a[TRANSIT_DRIVE],
            walk: a[WALK],
            bike: a[BIKE],
        }
    }
    /// Array indexed by the `params::DRIVE..BIKE` constants.
    pub fn to_array(self) -> [f64; N_CHOICES] {
        [
            self.drive,
            self.carpool,
            self.transit_walk,
            self.transit_drive,
            self.walk,
            self.bike,
        ]
    }
    /// Sum over all choices.
    pub fn total(&self) -> f64 {
        self.to_array().iter().sum()
    }
    /// Transit with walk access plus transit with drive access.
    pub fn transit(&self) -> f64 {
        self.transit_walk + self.transit_drive
    }
    /// Each entry divided by the total.
    pub fn shares(&self) -> ModeSplit {
        let t = self.total().max(1e-12);
        ModeSplit::from_array(self.to_array().map(|x| x / t))
    }
}

/// Ridership and economics for one line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineResult {
    /// Stable id returned by `Sim::add_line`.
    pub id: LineId,
    /// Line name as given in `TransitLine::name`.
    pub name: String,
    /// Transit technology.
    pub mode: Mode,
    /// Route length (stop-to-stop distance x alignment factor), km.
    pub length_km: f64,
    /// One-way end-to-end time including dwell, minutes.
    pub run_min: f64,
    /// Boardings per day, transfers included (one rider who transfers counts twice).
    pub daily_boardings: f64,
    /// Daily boardings at each stop, in stop order.
    pub boardings_by_stop: Vec<f64>,
    /// Passenger-km per day.
    pub passenger_km: f64,
    /// Highest daily load on any segment and direction.
    pub max_segment_load: f64,
    /// That load in the peak hour, vs. capacity per direction per hour.
    pub peak_hour_load: f64,
    /// Passengers per hour per direction the line can carry: 60 / headway x vehicle capacity.
    pub peak_hour_capacity: f64,
    /// Vehicles (trains) needed to run the headway: round trip x 1.1 layover / headway, rounded up.
    pub vehicles_required: f64,
    /// Vehicle revenue hours per day: vehicles x span.
    pub revenue_hours_per_day: f64,
    /// Operating cost per day, dollars: revenue hours x mode cost per revenue hour.
    pub operating_cost_per_day: f64,
    /// Fare revenue per day, dollars (boardings x fare + passenger-km x fare per km; overstates when transfers are free).
    pub fare_revenue_per_day: f64,
    /// Fare revenue / operating cost.
    pub farebox_recovery: f64,
    /// Daily passengers per segment: [forward (stop k -> k+1), reverse].
    pub segment_loads: Vec<[f64; 2]>,
}

/// Region-wide results after the latest evaluation. Serialized to the app as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Results {
    /// Person trips per day by choice.
    pub daily_trips: ModeSplit,
    /// Work (home-based work) trips, both directions.
    pub work_trips: ModeSplit,
    /// Non-work trips.
    pub other_trips: ModeSplit,
    /// `daily_trips` as shares of the total.
    pub mode_share: ModeSplit,
    /// Trips with no available option (no car, no transit, too far to walk).
    pub unserved_trips: f64,
    /// One entry per active line, in id order.
    pub lines: Vec<LineResult>,
    /// Peak-hour vehicle-km on the road network.
    pub peak_vehicle_km: f64,
    /// Peak-hour vehicle-km weighted mean speed.
    pub peak_avg_speed_kph: f64,
    /// Share of commuters whose desired arrival falls in the busiest hour.
    pub work_peak_hour_share: f64,
    /// True after a line edit until the next `equilibrate` (car times not re-run).
    pub car_times_stale: bool,
    /// Wall time of the last evaluation, milliseconds.
    pub eval_ms: f64,
}

/// Per-zone results, for heatmaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneResult {
    /// Zone id from the input.
    pub zone: u32,
    /// Daily trips starting in the zone.
    pub trips: f64,
    /// Daily transit trips starting in the zone.
    pub transit_trips: f64,
    /// `transit_trips / trips`.
    pub transit_share: f64,
    /// Share of the zone within walking range of a stop.
    pub stop_coverage: f64,
}

/// Peak-hour flow on one directed road edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkFlow {
    /// Index into `ScenarioInput::road_links`.
    pub link: u32,
    /// Start node id.
    pub from: u32,
    /// End node id.
    pub to: u32,
    /// Peak-hour vehicles per hour.
    pub volume_vph: f64,
    /// Volume / capacity.
    pub vc: f64,
    /// Congested speed, km/h.
    pub speed_kph: f64,
}

/// A zone pair and its daily travel, for the desire-line view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesireLine {
    /// First zone id.
    pub from: u32,
    /// Second zone id.
    pub to: u32,
    /// Daily trips both directions, all modes.
    pub trips: f64,
    /// Daily transit trips, both directions.
    pub transit_trips: f64,
}

struct CarSkim {
    time: Vec<f32>,
    km: Vec<f32>,
    toll: Vec<f32>,
}

/// One origin's mode-choice output.
struct Row {
    car_peak: Vec<f64>,
    wt_work: Vec<f64>,
    dt_work: Vec<f64>,
    wt_other: Vec<f64>,
    dt_other: Vec<f64>,
    work: [f64; N_CHOICES],
    other: [f64; N_CHOICES],
    unserved: f64,
}

/// The simulation. Owns the scenario, the networks, the demand and the latest results.
///
/// Build with `Sim::new`, edit lines with `add_line` / `update_line` / `remove_line`
/// (fast), advance time with `tick` (re-equilibrates once per game day).
pub struct Sim {
    /// Model parameters. After changing them call `equilibrate`.
    pub params: Params,
    /// The one game clock. `tick` advances it.
    pub clock: Clock,
    input: ScenarioInput,
    nz: usize,
    graph: RoadGraph,
    zone_node: Vec<u32>,
    zpts: Vec<ZonePt>,
    intra_km: Vec<f64>,
    vot_min: Vec<f64>,
    /// Daily home->work trips (AM direction), presence applied. Row-major Z x Z.
    hbw: Vec<f64>,
    /// Daily non-work trips, origin->destination.
    hbo: Vec<f64>,
    link_time: Vec<f64>,
    link_vol: Vec<f64>,
    car_free: CarSkim,
    car_peak: CarSkim,
    lines: Vec<Option<TransitLine>>,
    net: TransitNet,
    tsk_walk: Vec<TOd>,
    tsk_drive: Vec<TOd>,
    /// Daily transit trips i->j (walk + drive access), for desire lines and zone stats.
    transit_od: Vec<f64>,
    zone_trips: Vec<f64>,
    results: Results,
}

impl Sim {
    /// Build and run the baseline to equilibrium. Takes a few seconds for a county.
    pub fn new(input: ScenarioInput, params: Params) -> Result<Sim, SimError> {
        input.validate()?;
        let mut input = input;
        let initial_lines = std::mem::take(&mut input.lines);
        let nz = input.zones.len();
        let graph = RoadGraph::build(&input);
        let zone_node: Vec<u32> = input
            .zones
            .iter()
            .map(|z| graph.index[&z.road_node])
            .collect();
        let zindex: std::collections::HashMap<u32, usize> = input
            .zones
            .iter()
            .enumerate()
            .map(|(i, z)| (z.id, i))
            .collect();

        // Zone size: given area, else ~0.56 x spacing to the nearest neighbour
        // (radius of a square cell's equal-area disk).
        let zones = &input.zones;
        let zpts: Vec<ZonePt> = (0..nz)
            .into_par_iter()
            .map(|i| {
                let z = &zones[i];
                let r = match z.area_km2 {
                    Some(a) => (a / std::f64::consts::PI).sqrt(),
                    None => {
                        let nn = (0..nz)
                            .filter(|&j| j != i)
                            .map(|j| dist_km(z.lon, z.lat, zones[j].lon, zones[j].lat))
                            .fold(f64::INFINITY, f64::min);
                        if nn.is_finite() {
                            0.56 * nn
                        } else {
                            1.0
                        }
                    }
                };
                ZonePt {
                    lon: z.lon,
                    lat: z.lat,
                    radius_km: r,
                }
            })
            .collect();
        let intra_km = zpts.iter().map(|z| (0.9 * z.radius_km).max(0.3)).collect();
        let vot_min = zones
            .iter()
            .map(|z| {
                (params.vot_wage_share * z.median_income / 2080.0).max(params.vot_min_per_hour)
                    / 60.0
            })
            .collect();

        let mut hbw = vec![0.0; nz * nz];
        for c in &input.commutes {
            hbw[zindex[&c.home] * nz + zindex[&c.work]] += c.workers * params.commute_presence;
        }

        let link_time = graph.free_flow_times();
        let n_edges = graph.edges.len();
        let mut sim = Sim {
            params,
            clock: Clock::default(),
            nz,
            zone_node,
            zpts,
            intra_km,
            vot_min,
            hbw,
            hbo: Vec::new(),
            link_vol: vec![0.0; n_edges],
            car_free: CarSkim {
                time: vec![],
                km: vec![],
                toll: vec![],
            },
            car_peak: CarSkim {
                time: vec![],
                km: vec![],
                toll: vec![],
            },
            link_time,
            graph,
            input,
            lines: initial_lines.into_iter().map(Some).collect(),
            net: TransitNet::build(&[], &[], &Params::default()),
            tsk_walk: vec![],
            tsk_drive: vec![],
            transit_od: vec![0.0; nz * nz],
            zone_trips: vec![0.0; nz],
            results: Results::default(),
        };
        sim.car_free = sim.skim_car(&sim.link_time.clone());
        sim.hbo = sim.gravity_other();
        sim.rebuild_transit();
        sim.equilibrate();
        Ok(sim)
    }

    /// Parse a JSON `ScenarioInput` and build the `Sim` (see `new`).
    pub fn from_json(json: &str, params: Params) -> Result<Sim, SimError> {
        Sim::new(ScenarioInput::from_json(json)?, params)
    }

    // ---------------------------------------------------------------- API

    /// Latest results; refreshed by every call that changes state.
    pub fn results(&self) -> &Results {
        &self.results
    }

    /// The scenario as loaded. Its `lines` field is empty: lines live in `lines()`.
    pub fn input(&self) -> &ScenarioInput {
        &self.input
    }

    /// Active lines with their ids.
    pub fn lines(&self) -> Vec<(LineId, &TransitLine)> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|l| (i as LineId, l)))
            .collect()
    }

    /// Add a line and re-evaluate ridership (car times held fixed). Target < 2 s per county.
    pub fn add_line(&mut self, line: TransitLine) -> Result<LineId, SimError> {
        line.validate()?;
        self.lines.push(Some(line));
        self.on_lines_changed();
        Ok((self.lines.len() - 1) as LineId)
    }

    /// Replace line `id` in place (same id) and re-evaluate. Errors if the id is unknown or removed.
    pub fn update_line(&mut self, id: LineId, line: TransitLine) -> Result<(), SimError> {
        line.validate()?;
        match self.lines.get_mut(id as usize) {
            Some(slot @ Some(_)) => *slot = Some(line),
            _ => return Err(SimError(format!("no line {id}"))),
        }
        self.on_lines_changed();
        Ok(())
    }

    /// Remove line `id` and re-evaluate. The id is retired, never reused.
    pub fn remove_line(&mut self, id: LineId) -> Result<(), SimError> {
        match self.lines.get_mut(id as usize) {
            Some(slot @ Some(_)) => *slot = None,
            _ => return Err(SimError(format!("no line {id}"))),
        }
        self.on_lines_changed();
        Ok(())
    }

    /// Advance the shared clock by `real_dt` real seconds. Each game-day boundary
    /// re-runs the full equilibrium (strategic tier runs once per game day).
    /// Returns the number of days crossed. Call from a worker thread.
    pub fn tick(&mut self, real_dt: f64) -> u64 {
        let days = self.clock.advance(real_dt);
        if days > 0 {
            self.equilibrate();
        }
        days
    }

    /// Full run: mode choice <-> congested car assignment to a stable state, then
    /// transit loading. Seconds per county; the app calls it at day boundaries.
    pub fn equilibrate(&mut self) {
        let t0 = Instant::now();
        let k = self.params.feedback_iterations.max(1);
        self.link_time = self.graph.free_flow_times();
        self.link_vol.iter_mut().for_each(|v| *v = 0.0);
        for n in 1..=k {
            self.car_peak = self.skim_car(&self.link_time);
            self.transit_skims();
            let rows = self.choose_all();
            let demand = Self::flatten(&rows, |r| &r.car_peak);
            let aon = self.assign_car(&self.link_time, &demand);
            let step = 1.0 / n as f64;
            for (v, a) in self.link_vol.iter_mut().zip(&aon) {
                *v += step * (a - *v);
            }
            self.update_link_times();
        }
        self.car_peak = self.skim_car(&self.link_time);
        self.transit_skims();
        self.finish(t0, false);
    }

    /// Re-fit the alternative-specific constants so the model reproduces
    /// `targets` (any scale; normalized). Choices with a zero target or no
    /// modeled trips are left alone. Uses current skims; cheap.
    pub fn calibrate_mode_shares(&mut self, targets: ModeSplit, iterations: usize) {
        let tgt = targets.shares().to_array();
        for _ in 0..iterations {
            let rows = self.choose_all();
            let mut tot = [0.0; N_CHOICES];
            for r in &rows {
                for (m, t) in tot.iter_mut().enumerate() {
                    *t += 2.0 * r.work[m] + r.other[m];
                }
            }
            let sum: f64 = tot.iter().sum();
            for m in 0..N_CHOICES {
                if m == DRIVE || tgt[m] <= 0.0 || tot[m] <= 0.0 {
                    continue;
                }
                self.params.asc[m] +=
                    (tgt[m] / (tot[m] / sum)).ln() - (tgt[DRIVE] / (tot[DRIVE] / sum)).ln();
            }
        }
        self.finish(Instant::now(), self.results.car_times_stale);
    }

    /// Per-zone trips, transit trips and stop coverage.
    pub fn zone_results(&self) -> Vec<ZoneResult> {
        (0..self.nz)
            .map(|i| {
                let tr: f64 = self.transit_od[i * self.nz..(i + 1) * self.nz].iter().sum();
                ZoneResult {
                    zone: self.input.zones[i].id,
                    trips: self.zone_trips[i],
                    transit_trips: tr,
                    transit_share: if self.zone_trips[i] > 0.0 {
                        tr / self.zone_trips[i]
                    } else {
                        0.0
                    },
                    stop_coverage: self.net.coverage[i],
                }
            })
            .collect()
    }

    /// Peak-hour flows on every directed road edge.
    pub fn link_flows(&self) -> Vec<LinkFlow> {
        self.graph
            .edges
            .iter()
            .enumerate()
            .map(|(e, ed)| LinkFlow {
                link: ed.link,
                from: self.input.road_nodes[ed.from as usize].id,
                to: self.input.road_nodes[ed.to as usize].id,
                volume_vph: self.link_vol[e],
                vc: self.link_vol[e] / ed.cap_vph,
                speed_kph: ed.len_km / (self.link_time[e] / 60.0),
            })
            .collect()
    }

    /// Top `n` zone pairs by daily trips (both directions, all modes, excluding intrazonal).
    pub fn desire_lines(&self, n: usize) -> Vec<DesireLine> {
        let z = self.nz;
        let mut out = Vec::new();
        for i in 0..z {
            for j in (i + 1)..z {
                let (a, b) = (i * z + j, j * z + i);
                let trips = 2.0 * (self.hbw[a] + self.hbw[b]) + self.hbo[a] + self.hbo[b];
                if trips > 0.0 {
                    out.push(DesireLine {
                        from: self.input.zones[i].id,
                        to: self.input.zones[j].id,
                        trips,
                        transit_trips: self.transit_od[a] + self.transit_od[b],
                    });
                }
            }
        }
        out.sort_by(|a, b| b.trips.partial_cmp(&a.trips).unwrap());
        out.truncate(n);
        out
    }

    // ----------------------------------------------------------- internals

    fn on_lines_changed(&mut self) {
        let t0 = Instant::now();
        self.rebuild_transit();
        self.transit_skims();
        self.finish(t0, true);
    }

    fn rebuild_transit(&mut self) {
        let active: Vec<(u32, &TransitLine)> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|l| (i as u32, l)))
            .collect();
        self.net = TransitNet::build(&active, &self.zpts, &self.params);
    }

    /// Mode choice on current skims, transit loading, and results.
    fn finish(&mut self, t0: Instant, stale: bool) {
        let rows = self.choose_all();
        let z = self.nz;
        let wt_work = Self::flatten(&rows, |r| &r.wt_work);
        let dt_work = Self::flatten(&rows, |r| &r.dt_work);
        let wt_other = Self::flatten(&rows, |r| &r.wt_other);
        let dt_other = Self::flatten(&rows, |r| &r.dt_other);
        // Daily transit trips: AM commute i->j, PM return j->i, plus other trips.
        // Park-and-ride returns are loaded on the AM path (see MODEL.md).
        let mut d_walk = vec![0.0; z * z];
        let mut d_drive = vec![0.0; z * z];
        for i in 0..z {
            for j in 0..z {
                let k = i * z + j;
                d_walk[k] = wt_work[k] + wt_work[j * z + i] + wt_other[k];
                d_drive[k] = 2.0 * dt_work[k] + dt_other[k];
            }
        }
        let loads = self.load_transit(&d_walk, &d_drive);
        for k in 0..z * z {
            self.transit_od[k] = d_walk[k] + d_drive[k];
        }

        let mut work = [0.0; N_CHOICES];
        let mut other = [0.0; N_CHOICES];
        let mut unserved = 0.0;
        for (i, r) in rows.iter().enumerate() {
            let mut zt = 0.0;
            for m in 0..N_CHOICES {
                work[m] += 2.0 * r.work[m];
                other[m] += r.other[m];
                zt += 2.0 * r.work[m] + r.other[m];
            }
            self.zone_trips[i] = zt;
            unserved += r.unserved;
        }
        let work = ModeSplit::from_array(work);
        let other = ModeSplit::from_array(other);
        let daily = ModeSplit::from_array(std::array::from_fn(|m| {
            work.to_array()[m] + other.to_array()[m]
        }));

        // Share of a line's daily load that falls in the peak hour: AM work trips
        // in the busiest arrival hour plus the off-peak share of other trips.
        let wph = self.params.work_peak_hour_share();
        let transit_peak_factor = if daily.transit() > 0.0 {
            (work.transit() / 2.0 * wph + other.transit() * self.params.other_peak_hour_share)
                / daily.transit()
        } else {
            0.0
        };
        let lines = self.line_results(&loads, transit_peak_factor);

        let (mut vkm, mut vhr) = (0.0, 0.0);
        for (e, ed) in self.graph.edges.iter().enumerate() {
            vkm += self.link_vol[e] * ed.len_km;
            vhr += self.link_vol[e] * self.link_time[e] / 60.0;
        }
        self.results = Results {
            daily_trips: daily,
            work_trips: work,
            other_trips: other,
            mode_share: daily.shares(),
            unserved_trips: unserved,
            lines,
            peak_vehicle_km: vkm,
            peak_avg_speed_kph: if vhr > 0.0 { vkm / vhr } else { 0.0 },
            work_peak_hour_share: wph,
            car_times_stale: stale,
            eval_ms: t0.elapsed().as_secs_f64() * 1000.0,
        };
    }

    fn line_results(&self, loads: &LineLoads, peak_factor: f64) -> Vec<LineResult> {
        self.net
            .lines
            .iter()
            .enumerate()
            .map(|(li, l)| {
                let max_seg = loads.seg[li]
                    .iter()
                    .flat_map(|s| s.iter().copied())
                    .fold(0.0, f64::max);
                // Round trip with 10% layover, rounded up to whole vehicles.
                let cycle = 2.0 * l.run_min() * 1.1;
                let vehicles = (cycle / l.headway).ceil().max(1.0);
                let rev_hours = vehicles * l.span_hours;
                let op_cost = rev_hours * l.defaults.cost_per_revenue_hour;
                let revenue = loads.boardings[li] * l.fare + loads.pkm[li] * l.fare_per_km;
                LineResult {
                    id: l.id,
                    name: l.name.clone(),
                    mode: l.mode,
                    length_km: l.length_km(),
                    run_min: l.run_min(),
                    daily_boardings: loads.boardings[li],
                    boardings_by_stop: loads.stop_boardings[li].clone(),
                    passenger_km: loads.pkm[li],
                    max_segment_load: max_seg,
                    peak_hour_load: max_seg * peak_factor,
                    peak_hour_capacity: 60.0 / l.headway * l.defaults.vehicle_capacity,
                    vehicles_required: vehicles,
                    revenue_hours_per_day: rev_hours,
                    operating_cost_per_day: op_cost,
                    fare_revenue_per_day: revenue,
                    farebox_recovery: if op_cost > 0.0 {
                        revenue / op_cost
                    } else {
                        0.0
                    },
                    segment_loads: loads.seg[li].clone(),
                }
            })
            .collect()
    }

    fn flatten(rows: &[Row], f: impl Fn(&Row) -> &Vec<f64>) -> Vec<f64> {
        let mut out = Vec::with_capacity(rows.len() * rows.len());
        for r in rows {
            out.extend_from_slice(f(r));
        }
        out
    }

    fn crow_km(&self, i: usize, j: usize) -> f64 {
        if i == j {
            return self.intra_km[i];
        }
        let (a, b) = (&self.zpts[i], &self.zpts[j]);
        dist_km(a.lon, a.lat, b.lon, b.lat)
    }

    fn skim_car(&self, times: &[f64]) -> CarSkim {
        let z = self.nz;
        let g = &self.graph;
        let rows: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = (0..z)
            .into_par_iter()
            .map_init(
                || {
                    (
                        Tree::new(g.n_nodes, &self.zone_node),
                        vec![0.0; g.n_nodes],
                        vec![0.0; g.n_nodes],
                    )
                },
                |(tree, km, toll), i| {
                    tree.run(g, times, self.zone_node[i]);
                    tree.km_and_toll(g, km, toll);
                    let mut t = vec![f32::INFINITY; z];
                    let mut k = vec![0f32; z];
                    let mut tl = vec![0f32; z];
                    for j in 0..z {
                        let n = self.zone_node[j] as usize;
                        if i == j || n == self.zone_node[i] as usize {
                            // Same zone or same loading node: local streets at ~25 km/h.
                            let d = self.crow_km(i, j).max(self.intra_km[i])
                                * self.params.detour_factor;
                            t[j] = (d / 25.0 * 60.0) as f32;
                            k[j] = d as f32;
                        } else if tree.dist[n].is_finite() {
                            t[j] = tree.dist[n] as f32;
                            k[j] = km[n] as f32;
                            tl[j] = toll[n] as f32;
                        }
                    }
                    (t, k, tl)
                },
            )
            .collect();
        let mut s = CarSkim {
            time: Vec::with_capacity(z * z),
            km: Vec::with_capacity(z * z),
            toll: Vec::with_capacity(z * z),
        };
        for (t, k, tl) in rows {
            s.time.extend(t);
            s.km.extend(k);
            s.toll.extend(tl);
        }
        s
    }

    /// All-or-nothing assignment of a Z x Z vehicle demand matrix on `times`.
    fn assign_car(&self, times: &[f64], demand: &[f64]) -> Vec<f64> {
        let z = self.nz;
        let g = &self.graph;
        let ne = g.edges.len();
        (0..z)
            .into_par_iter()
            .fold(
                || {
                    (
                        Tree::new(g.n_nodes, &self.zone_node),
                        vec![0.0; g.n_nodes],
                        vec![0.0; ne],
                    )
                },
                |(mut tree, mut nd, mut vol), i| {
                    let row = &demand[i * z..(i + 1) * z];
                    if row.iter().any(|&d| d > 0.0) {
                        tree.run(g, times, self.zone_node[i]);
                        for (j, &d) in row.iter().enumerate() {
                            nd[self.zone_node[j] as usize] += d;
                        }
                        nd[self.zone_node[i] as usize] = 0.0; // same-node trips stay off the network
                        tree.load(g, &mut nd, &mut vol);
                    }
                    (tree, nd, vol)
                },
            )
            .map(|(_, _, v)| v)
            .reduce(
                || vec![0.0; ne],
                |mut a, b| {
                    for (x, y) in a.iter_mut().zip(b) {
                        *x += y;
                    }
                    a
                },
            )
    }

    fn update_link_times(&mut self) {
        let (a, b) = (self.params.bpr_alpha, self.params.bpr_beta);
        for (e, ed) in self.graph.edges.iter().enumerate() {
            let vc = self.link_vol[e] / ed.cap_vph;
            self.link_time[e] = ed.t0_min * (1.0 + a * vc.powf(b));
        }
    }

    /// Doubly-unconstrained gravity model for non-work trips on free-flow car time.
    fn gravity_other(&self) -> Vec<f64> {
        let z = self.nz;
        let zones = &self.input.zones;
        let p = &self.params;
        let rows: Vec<Vec<f64>> = (0..z)
            .into_par_iter()
            .map(|i| {
                let prod = zones[i].population * p.other_trips_per_person;
                let mut w: Vec<f64> = (0..z)
                    .map(|j| {
                        let t = self.car_free.time[i * z + j] as f64;
                        let attr = zones[j].jobs + 0.5 * zones[j].population;
                        if t.is_finite() {
                            attr * (-p.gravity_decay_per_min * t).exp()
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let s: f64 = w.iter().sum();
                if s > 0.0 {
                    w.iter_mut().for_each(|x| *x *= prod / s);
                }
                w
            })
            .collect();
        rows.concat()
    }

    fn transit_skims(&mut self) {
        let z = self.nz;
        if self.net.is_empty() {
            self.tsk_walk = vec![TOd::NONE; z * z];
            self.tsk_drive = vec![TOd::NONE; z * z];
            return;
        }
        let (net, p, car) = (&self.net, &self.params, &self.car_peak);
        let rows: Vec<(Vec<TOd>, Vec<TOd>)> = (0..z)
            .into_par_iter()
            .map_init(
                || Searcher::new(net, p, z),
                |s, i| {
                    let (cm, ck) = (&car.time[i * z..(i + 1) * z], &car.km[i * z..(i + 1) * z]);
                    let mut w = vec![TOd::NONE; z];
                    let mut d = vec![TOd::NONE; z];
                    if !net.walk_access[i].is_empty() {
                        s.run(i, Access::Walk, cm, ck);
                        (0..z).for_each(|j| w[j] = s.od(j));
                    }
                    if !net.pnr.is_empty() {
                        s.run(i, Access::Drive, cm, ck);
                        (0..z).for_each(|j| d[j] = s.od(j));
                    }
                    (w, d)
                },
            )
            .collect();
        self.tsk_walk.clear();
        self.tsk_drive.clear();
        for (w, d) in rows {
            self.tsk_walk.extend(w);
            self.tsk_drive.extend(d);
        }
    }

    fn load_transit(&self, d_walk: &[f64], d_drive: &[f64]) -> LineLoads {
        let z = self.nz;
        let net = &self.net;
        if net.is_empty() {
            return LineLoads::new(net);
        }
        let (p, car) = (&self.params, &self.car_peak);
        (0..z)
            .into_par_iter()
            .fold(
                || (Searcher::new(net, p, z), LineLoads::new(net)),
                |(mut s, mut acc), i| {
                    let (cm, ck) = (&car.time[i * z..(i + 1) * z], &car.km[i * z..(i + 1) * z]);
                    for (access, d) in [(Access::Walk, d_walk), (Access::Drive, d_drive)] {
                        let row = &d[i * z..(i + 1) * z];
                        if row.iter().any(|&x| x > 0.0) {
                            s.run(i, access, cm, ck);
                            for (j, &x) in row.iter().enumerate() {
                                s.load(j, x, &mut acc);
                            }
                        }
                    }
                    (s, acc)
                },
            )
            .map(|(_, a)| a)
            .reduce(|| LineLoads::new(net), LineLoads::merge)
    }

    fn choose_all(&self) -> Vec<Row> {
        (0..self.nz)
            .into_par_iter()
            .map(|i| self.choose_row(i))
            .collect()
    }

    /// Generalized cost (minutes) of each choice for one OD pair, or None if unavailable.
    fn costs(&self, i: usize, j: usize, work: bool) -> [Option<f64>; N_CHOICES] {
        let p = &self.params;
        let k = i * self.nz + j;
        let vot = self.vot_min[i];
        let car = if work { &self.car_peak } else { &self.car_free };
        let mut c = [None; N_CHOICES];
        let t = car.time[k] as f64;
        if t.is_finite() {
            let share = if work {
                p.parking_share_work
            } else {
                p.parking_share_other
            };
            let money = car.km[k] as f64 * p.car_cost_per_km
                + car.toll[k] as f64
                + self.input.zones[j].parking_cost * share;
            c[DRIVE] = Some(t + p.car_terminal_min + money / vot);
            c[CARPOOL] = Some(
                t + p.car_terminal_min + p.carpool_extra_min + money / p.carpool_occupancy / vot,
            );
        }
        let transit = |o: &TOd| {
            let sd = if work {
                p.schedule_delay_weight * o.last_headway as f64 / 2.0
            } else {
                0.0
            };
            o.drive_min as f64
                + o.ivt as f64
                + p.wait_weight * o.wait as f64
                + p.walk_weight * o.walk_min as f64
                + p.transfer_penalty_min * o.xfers as f64
                + (o.fare + o.drive_cost) as f64 / vot
                + sd
        };
        let tw = &self.tsk_walk[k];
        if tw.ok() {
            c[TRANSIT_WALK] = Some(transit(tw));
        }
        let td = &self.tsk_drive[k];
        if td.ok() {
            c[TRANSIT_DRIVE] = Some(transit(td));
        }
        let d = self.crow_km(i, j) * p.detour_factor;
        if d <= p.max_walk_km {
            c[WALK] = Some(d / p.walk_kph * 60.0);
        }
        if d <= p.max_bike_km {
            c[BIKE] = Some(d / p.bike_kph * 60.0);
        }
        c
    }

    fn choose_row(&self, i: usize) -> Row {
        let z = self.nz;
        let p = &self.params;
        let mut r = Row {
            car_peak: vec![0.0; z],
            wt_work: vec![0.0; z],
            dt_work: vec![0.0; z],
            wt_other: vec![0.0; z],
            dt_other: vec![0.0; z],
            work: [0.0; N_CHOICES],
            other: [0.0; N_CHOICES],
            unserved: 0.0,
        };
        let zc = self.input.zones[i].zero_car_share;
        let cov_i = self.net.coverage[i];
        let wph = p.work_peak_hour_share();
        for j in 0..z {
            for work in [true, false] {
                let trips = if work {
                    self.hbw[i * z + j]
                } else {
                    self.hbo[i * z + j]
                };
                if trips <= 0.0 {
                    continue;
                }
                let c = self.costs(i, j, work);
                let near = cov_i * self.net.coverage[j];
                let mut prob = [0.0; N_CHOICES];
                // Four segments: car / no car x origin+destination near a stop or not.
                for (has_car, w_car) in [(true, 1.0 - zc), (false, zc)] {
                    for (is_near, w_near) in [(true, near), (false, 1.0 - near)] {
                        let w = w_car * w_near;
                        if w <= 0.0 {
                            continue;
                        }
                        let mut avail = c;
                        if !has_car {
                            avail[DRIVE] = None;
                            avail[TRANSIT_DRIVE] = None;
                        }
                        if !is_near {
                            avail[TRANSIT_WALK] = None;
                        }
                        match logit(&p.asc, p.beta_per_min, &avail) {
                            Some(pr) => (0..N_CHOICES).for_each(|m| prob[m] += w * pr[m]),
                            None => {
                                // Count unserved trips per day (work trips happen twice).
                                r.unserved += w * trips * if work { 2.0 } else { 1.0 };
                            }
                        }
                    }
                }
                let veh = prob[DRIVE] + prob[CARPOOL] / p.carpool_occupancy;
                if work {
                    (0..N_CHOICES).for_each(|m| r.work[m] += trips * prob[m]);
                    r.wt_work[j] = trips * prob[TRANSIT_WALK];
                    r.dt_work[j] = trips * prob[TRANSIT_DRIVE];
                    r.car_peak[j] += trips * veh * wph;
                } else {
                    (0..N_CHOICES).for_each(|m| r.other[m] += trips * prob[m]);
                    r.wt_other[j] = trips * prob[TRANSIT_WALK];
                    r.dt_other[j] = trips * prob[TRANSIT_DRIVE];
                    r.car_peak[j] += trips * veh * p.other_peak_hour_share;
                }
            }
        }
        r
    }
}

/// Multinomial logit probabilities; None if nothing is available.
pub fn logit(
    asc: &[f64; N_CHOICES],
    beta: f64,
    gc: &[Option<f64>; N_CHOICES],
) -> Option<[f64; N_CHOICES]> {
    let mut u = [f64::NEG_INFINITY; N_CHOICES];
    let mut max = f64::NEG_INFINITY;
    for m in 0..N_CHOICES {
        if let Some(c) = gc[m] {
            u[m] = asc[m] + beta * c;
            max = max.max(u[m]);
        }
    }
    if !max.is_finite() {
        return None;
    }
    let mut p = [0.0; N_CHOICES];
    let mut s = 0.0;
    for m in 0..N_CHOICES {
        if u[m].is_finite() {
            p[m] = (u[m] - max).exp();
            s += p[m];
        }
    }
    p.iter_mut().for_each(|x| *x /= s);
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logit_sums_to_one_and_respects_availability() {
        let asc = [0.0, -1.0, -1.0, -2.0, -1.0, -2.0];
        let gc = [Some(20.0), Some(25.0), None, None, Some(60.0), None];
        let p = logit(&asc, -0.035, &gc).unwrap();
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(p[TRANSIT_WALK], 0.0);
        assert!(p[DRIVE] > p[CARPOOL] && p[CARPOOL] > p[WALK]);
        assert!(logit(&asc, -0.035, &[None; N_CHOICES]).is_none());
    }
}
