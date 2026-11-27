//! The `Sim`: owns the scenario, runs demand -> mode choice -> assignment, and
//! answers the app's questions. See docs/MODEL.md for the math and the reasons
//! behind each simplification.

use crate::clock::Clock;
use crate::economy::{self, Ledger};
use crate::geo::dist_km;
use crate::graph::{RoadGraph, Tree, NONE};
use crate::pairs::Pairs;
use crate::params::*;
use crate::roads::Router;
use crate::schema::*;
use crate::transit::{Access, LineLoads, Searcher, TOd, TransitNet, ZonePt};
use crate::SimError;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// Cars parked per day at each stop's park-and-ride lot, in stop order
    /// (0 where the stop has no lot). Compare with `Stop::park_ride_spaces`.
    pub park_ride_cars_by_stop: Vec<f64>,
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
    /// Street route per segment for bus, trolleybus and BRT: [lon, lat]
    /// points from stop k to stop k+1 along the roads (SIM-8). An empty entry
    /// means the segment fell back to a straight line (a stop over 500 m from
    /// a road, or no path); the whole list is empty for other modes.
    pub segment_shapes: Vec<Vec<[f64; 2]>>,
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
    /// Daily work trips between zones beyond `Params::strategic_max_min`, not
    /// modeled by this tier (0 without a cutoff). Intercity travel is F37.
    #[serde(default)]
    pub beyond_cutoff_trips: f64,
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
    /// Lines under construction (see `Sim::pending_lines`).
    pub pending_lines: Vec<PendingLine>,
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

/// Transit skims per origin, aligned to `Pairs::row`. An empty row means no
/// transit to any destination (most statewide tracts), so it costs nothing
/// to store or to re-evaluate (SIM-17). An empty outer vector for the
/// off-peak skims means "same as peak".
type TSkim = Vec<Vec<TOd>>;

/// Saved road volumes for a warm start (`Sim::road_state`, `Sim::new_warm`,
/// SIM-19). `fingerprint` identifies the contracted road graph it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadState {
    /// `RoadGraph::fingerprint` of the graph the volumes belong to.
    pub fingerprint: u64,
    /// Peak-hour volume per contracted road edge, vehicles/hour.
    pub link_vol: Vec<f64>,
}

/// A game day's road re-run, spread over `tick_budgeted` calls (SIM-18):
/// round 0 re-prices park-and-ride, rounds 1..=`assign_rounds` load slices of
/// origins on the roads and step toward that load, and the last round skims
/// and chooses on the new car times and commits.
struct DayJob {
    round: usize,
    assign_rounds: usize,
    /// Next origin of the current pass.
    next: usize,
    vol: Vec<f64>,
    outs: Vec<(usize, OriginOut)>,
    /// `Sim::lines_version` when the final pass began.
    lines_at_start: u64,
}

impl DayJob {
    fn new(assign_rounds: usize) -> DayJob {
        DayJob {
            round: 0,
            assign_rounds,
            next: 0,
            vol: Vec::new(),
            outs: Vec::new(),
            lines_at_start: 0,
        }
    }

    fn progress(&self, z: usize) -> f64 {
        let passes = (self.assign_rounds + 1) as f64;
        if self.round == 0 {
            return 0.0;
        }
        ((self.round - 1) as f64 + self.next as f64 / z.max(1) as f64) / passes
    }
}

/// Transit-side state a line evaluation replaces (see `Sim::on_lines_changed`).
struct TransitState {
    net: TransitNet,
    tsk: [TSkim; 4],
    transit_od: Vec<f64>,
    zone_trips: Vec<f64>,
    results: Results,
    pnr_cars: Vec<f64>,
    pnr_penalty: HashMap<(LineId, usize), f64>,
    loads: LineLoads,
}

/// What a line edit can change (SIM-21). A transit search only explores the
/// components (see `TransitNet::comp`) its access platforms are in, so an
/// origin whose components are the same before and after the edit keeps
/// its skim row, and a line no re-searched origin reaches keeps its load.
struct EditScope {
    /// Origins to re-search; the rest keep their rows.
    search: Vec<bool>,
    /// Origins to re-load: those reaching a line whose load can change.
    load: Vec<bool>,
    /// Per line of the new net: its index in the old net when its load
    /// carries over unchanged.
    keep: Vec<Option<usize>>,
    /// Old platform index -> new, for park-and-ride ids in kept rows.
    plat_map: Vec<u32>,
}

/// Congested shortest-path trees stop at this multiple of the strategic
/// cutoff (SIM-14). A kept pair past it keeps its car option at the guard time
/// (SIM-15).
const PEAK_GUARD: f64 = 4.0;

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

/// One origin's skim rows (peak car and transit) for mode choice.
struct RowInputs<'a> {
    peak_t: &'a [f32],
    peak_km: &'a [f32],
    peak_toll: &'a [f32],
    tw: &'a [TOd],
    td: &'a [TOd],
    /// Off-peak transit components (non-work trips); same as `tw`/`td` when
    /// no line runs a different off-peak headway.
    tw_off: &'a [TOd],
    td_off: &'a [TOd],
}

/// Everything one origin produces in a fused pass.
struct OriginOut {
    t: Vec<f32>,
    k: Vec<f32>,
    tl: Vec<f32>,
    w: Vec<TOd>,
    d: Vec<TOd>,
    w_off: Vec<TOd>,
    d_off: Vec<TOd>,
    row: Row,
}

/// Per-origin mode totals (work, other, unserved) of an origin with no
/// transit, whose choices depend only on car times and params.
type IdleRow = ([f64; N_CHOICES], [f64; N_CHOICES], f64);

/// One origin's mode-choice output. The per-pair vectors are empty for an
/// origin reused from `Sim::idle_rows` (no transit, nothing to load).
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
    /// Kept zone pairs (all of them without a strategic cutoff). Every
    /// per-pair vector below is indexed by pair (F34).
    pairs: Pairs,
    /// Daily work trips (both directions) beyond the cutoff, by home zone.
    beyond_cutoff: Vec<f64>,
    /// Daily home->work trips (AM direction), presence applied, per pair.
    hbw: Vec<f64>,
    /// Daily non-work trips, origin->destination.
    hbo: Vec<f64>,
    link_time: Vec<f64>,
    link_vol: Vec<f64>,
    car_free: CarSkim,
    car_peak: CarSkim,
    lines: Vec<Option<TransitLine>>,
    net: TransitNet,
    tsk_walk: TSkim,
    tsk_drive: TSkim,
    /// Off-peak transit skims; empty when no line has its own off-peak headway.
    tsk_walk_off: TSkim,
    tsk_drive_off: TSkim,
    /// `TOd::NONE` x the longest row: what an origin with no transit row reads.
    none_row: Vec<TOd>,
    /// Daily transit trips i->j (walk + drive access), for desire lines and zone stats.
    transit_od: Vec<f64>,
    zone_trips: Vec<f64>,
    results: Results,
    ledger: Ledger,
    /// Capital already paid per line slot, so an extension pays only the difference.
    capital_paid: Vec<f64>,
    /// Per line slot: a version under construction and the game day it opens.
    /// `lines` holds the version in service (None while a new line is built).
    pending: Vec<Option<(TransitLine, u64)>>,
    /// Road snapping and route cache for bus, trolleybus and BRT (SIM-8).
    router: Router,
    /// Park-and-ride shadow price per (line, stop index), generalized minutes
    /// (SIM-11). Kept by stop, not platform, so it survives line edits.
    pnr_penalty: HashMap<(LineId, usize), f64>,
    /// Cars parked per platform on the last loading (drive-access commuters
    /// plus half the drive-access other trips).
    pnr_cars: Vec<f64>,
    /// Daily loads per line of `net` from the last loading (peak and
    /// off-peak merged), carried over for lines an edit cannot reach (SIM-21).
    loads: LineLoads,
    /// Mode-choice totals of origins no transit reaches, reused by line
    /// edits while car times and params are unchanged (SIM-17). Keyed by
    /// the params as JSON; `pass` clears the key when car times move.
    idle_rows: Vec<Option<IdleRow>>,
    idle_key: Option<String>,
    /// A game day's re-run in progress (`tick_budgeted`, SIM-18).
    day_job: Option<DayJob>,
    /// Bumped on every line change, so a day job can tell its transit rows
    /// went stale.
    lines_version: u64,
    started_warm: bool,
}

/// A line (or a new version of one) under construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLine {
    /// Line id.
    pub id: LineId,
    /// Display name.
    pub name: String,
    /// Transit technology.
    pub mode: Mode,
    /// Game day (`Clock::day`) the new version enters service.
    pub opens_day: u64,
    /// True if an older version of the line keeps running until then.
    pub replaces_open_line: bool,
}

/// What a line would cost and carry, from `Sim::preview_line`. Nothing is charged.
#[derive(Debug, Clone, Serialize)]
pub struct LinePreview {
    /// Ridership and operating economics as if the line were built today.
    pub line: LineResult,
    /// Capital cost: guideway per km plus vehicles (see `economy::capital_cost`).
    pub capital_cost: f64,
    /// Federal grant it would receive (0 if it fails the cost-per-rider gate).
    pub federal_grant: f64,
    /// Capital cost per forecast daily boarding, the grant gate's measure.
    pub capital_per_daily_boarding: f64,
}

impl Sim {
    /// Build and run the baseline to equilibrium. Takes a few seconds for a county.
    pub fn new(input: ScenarioInput, params: Params) -> Result<Sim, SimError> {
        Sim::build(input, params, None)
    }

    /// `new`, starting the roads from a saved [`RoadState`] (`road_state()`
    /// of an earlier game on the same scenario) instead of free flow, so the
    /// cold equilibrium becomes one warm game day (SIM-19). A state from
    /// another road network (fingerprint mismatch) is ignored and the start
    /// is cold, as with `new`.
    pub fn new_warm(
        input: ScenarioInput,
        params: Params,
        roads: &RoadState,
    ) -> Result<Sim, SimError> {
        Sim::build(input, params, Some(roads))
    }

    /// Road volumes to save and pass to `new_warm` next time. Small: one
    /// number per contracted road edge (~3.5 MB statewide as JSON).
    pub fn road_state(&self) -> RoadState {
        RoadState {
            fingerprint: self.graph.fingerprint(),
            link_vol: self.link_vol.clone(),
        }
    }

    /// Whether the last build started warm from a saved `RoadState`.
    pub fn started_warm(&self) -> bool {
        self.started_warm
    }

    fn build(
        input: ScenarioInput,
        params: Params,
        roads: Option<&RoadState>,
    ) -> Result<Sim, SimError> {
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

        let link_time = graph.free_flow_times();
        let pairs = match params.strategic_max_min {
            None => Pairs::dense(nz),
            Some(cut) => {
                let rows = (0..nz)
                    .into_par_iter()
                    .map_init(
                        || {
                            let mut t = Tree::new(graph.n_nodes, &zone_node);
                            t.bound = cut;
                            t
                        },
                        |tree, i| {
                            tree.run(&graph, &link_time, zone_node[i]);
                            (0..nz)
                                .filter(|&j| tree.reached(zone_node[j] as usize))
                                .map(|j| j as u32)
                                .collect::<Vec<u32>>()
                        },
                    )
                    .collect();
                Pairs::from_rows(rows)
            }
        };
        let mut hbw = vec![0.0; pairs.len()];
        let mut beyond_cutoff = vec![0.0; nz];
        for c in &input.commutes {
            let w = c.workers * params.commute_presence;
            let h = zindex[&c.home];
            match pairs.find(h, zindex[&c.work]) {
                Some(k) => hbw[k] += w,
                None => beyond_cutoff[h] += 2.0 * w,
            }
        }

        let n_edges = graph.edges.len();
        let mut sim = Sim {
            params,
            clock: Clock::default(),
            nz,
            zone_node,
            zpts,
            intra_km,
            vot_min,
            beyond_cutoff,
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
            net: TransitNet::build(&[], &[], &Params::default(), &mut |_, _| None),
            router: Router::default(),
            pnr_penalty: HashMap::new(),
            pnr_cars: vec![],
            loads: LineLoads::default(),
            idle_rows: vec![],
            idle_key: None,
            day_job: None,
            lines_version: 0,
            started_warm: false,
            tsk_walk: vec![],
            tsk_drive: vec![],
            tsk_walk_off: vec![],
            tsk_drive_off: vec![],
            none_row: vec![TOd::NONE; (0..nz).map(|i| pairs.row(i).len()).max().unwrap_or(0)],
            transit_od: vec![0.0; pairs.len()],
            pairs,
            zone_trips: vec![0.0; nz],
            results: Results::default(),
            ledger: Ledger::new(0.0, 0),
            capital_paid: vec![],
            pending: vec![],
        };
        sim.ledger = Ledger::new(sim.params.starting_cash, sim.clock.month());
        sim.router = Router::new(&sim.input, &sim.graph);
        sim.car_free = sim.skim_car(&sim.link_time.clone());
        sim.hbo = sim.gravity_other();
        sim.rebuild_transit();
        match roads {
            Some(r)
                if r.fingerprint == sim.graph.fingerprint()
                    && r.link_vol.len() == sim.link_vol.len()
                    && r.link_vol.iter().all(|v| v.is_finite() && *v >= 0.0) =>
            {
                sim.link_vol.clone_from(&r.link_vol);
                sim.update_link_times();
                sim.started_warm = true;
                sim.equilibrate_warm();
            }
            _ => sim.equilibrate(),
        }
        // Lines present at start are already paid for.
        sim.capital_paid = (0..sim.lines.len())
            .map(|i| sim.line_capital(i as LineId).unwrap_or(0.0))
            .collect();
        sim.pending = vec![None; sim.lines.len()];
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

    /// Every line with its id, as the player last drew it: the version under
    /// construction if there is one, else the version in service.
    pub fn lines(&self) -> Vec<(LineId, &TransitLine)> {
        self.lines
            .iter()
            .zip(&self.pending)
            .enumerate()
            .filter_map(|(i, (l, p))| {
                p.as_ref()
                    .map(|p| &p.0)
                    .or(l.as_ref())
                    .map(|l| (i as LineId, l))
            })
            .collect()
    }

    /// Lines under construction and the day each opens.
    pub fn pending_lines(&self) -> Vec<PendingLine> {
        self.pending
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                p.as_ref().map(|(l, day)| PendingLine {
                    id: i as LineId,
                    name: l.name.clone(),
                    mode: l.mode,
                    opens_day: *day,
                    replaces_open_line: self.lines[i].is_some(),
                })
            })
            .collect()
    }

    /// Game day a line drawn today would open: real-world build time for its
    /// mode divided by `construction_speedup`. Today if `instant_build`.
    pub fn opening_day(&self, mode: Mode) -> u64 {
        if self.params.instant_build {
            return self.clock.day();
        }
        let days = self
            .params
            .build_days(mode_defaults(mode).build_years * 365.0);
        self.clock.day() + days.ceil().max(0.0) as u64
    }

    /// Add a line. Its capital is charged now (less any federal grant on its
    /// forecast ridership) and it enters service on `opening_day(mode)`; until
    /// then it is in `pending_lines()` and carries no riders. With
    /// `params.instant_build` it opens at once and ridership is re-evaluated
    /// (car times held fixed). Target < 2 s per county.
    pub fn add_line(&mut self, line: TransitLine) -> Result<LineId, SimError> {
        line.validate()?;
        let opens = self.opening_day(line.mode);
        let id = self.lines.len() as LineId;
        self.capital_paid.push(0.0);
        self.pending.push(None);
        if opens <= self.clock.day() {
            self.lines.push(Some(line));
            self.on_lines_changed();
            self.charge_capital(id);
        } else {
            self.lines.push(None);
            let (r, capital) = self.evaluate_in_slot(id, &line)?;
            self.charge(id, capital, r.daily_boardings);
            self.pending[id as usize] = Some((line, opens));
            self.refresh_pending();
        }
        Ok(id)
    }

    /// Evaluate `line` as slot `id`'s version in service, read its result and
    /// capital cost, then put the slot back. Leaves results as they were.
    fn evaluate_in_slot(
        &mut self,
        id: LineId,
        line: &TransitLine,
    ) -> Result<(LineResult, f64), SimError> {
        let old = self.lines[id as usize].replace(line.clone());
        let saved = self.on_lines_changed();
        let r = self.line_result(id).cloned();
        let capital = self.line_capital(id);
        self.lines[id as usize] = old;
        // Same lines and car times as before: put the previous transit state
        // back instead of re-evaluating it (half the cost of an edit).
        self.restore_transit(saved);
        r.zip(capital)
            .ok_or_else(|| SimError("line evaluation failed".into()))
    }

    /// Cost and ridership of `line` without building it: evaluates it as a new
    /// line, then removes it again (two line edits, ~40-80 ms). The ledger and
    /// line ids are unchanged.
    pub fn preview_line(&mut self, line: TransitLine) -> Result<LinePreview, SimError> {
        line.validate()?;
        let id = self.lines.len() as LineId;
        self.lines.push(None);
        let r = self.evaluate_in_slot(id, &line);
        self.lines.pop();
        let (line, capital_cost) = r?;
        let federal_grant = economy::federal_grant(
            capital_cost,
            capital_cost,
            line.daily_boardings,
            &self.params,
        );
        Ok(LinePreview {
            capital_per_daily_boarding: capital_cost / line.daily_boardings.max(1e-9),
            line,
            capital_cost,
            federal_grant,
        })
    }

    /// The transit authority's books: cash, the month in progress, settled months.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Annual local tax the authority will raise once every line now open or
    /// under construction is running: the tax area of all their stops
    /// (SIM-9). This is the revenue pledged to its bonds, so building lines
    /// raises what it may borrow.
    pub fn tax_capacity(&self) -> f64 {
        let mut stops: Vec<[f64; 2]> = self.net.platforms.iter().map(|s| [s.lon, s.lat]).collect();
        for (l, _) in self.pending.iter().flatten() {
            stops.extend(l.stops.iter().map(|s| [s.lon, s.lat]));
        }
        self.taxed_population(&stops)
            * self.params.taxable_sales_per_capita
            * self.params.local_sales_tax_rate
    }

    /// Most principal a new bond may borrow now under the additional-bonds
    /// test: annual debt service, old plus new, at most `tax_capacity() /
    /// bond_coverage_min`. 0 once bankrupt.
    pub fn bond_capacity(&self) -> f64 {
        if self.ledger.bankrupt_month.is_some() {
            return 0.0;
        }
        let room = self.tax_capacity() / self.params.bond_coverage_min.max(1e-9) / 12.0
            - self.ledger.monthly_debt_service();
        let per_dollar = economy::Bond::new(
            1.0,
            self.params.bond_rate,
            self.params.bond_term_months(),
            0,
        )
        .payment;
        (room / per_dollar).max(0.0)
    }

    /// Months until the last line now under construction opens (0 if none).
    fn construction_months_left(&self) -> u64 {
        let today = self.clock.day();
        let days = self
            .pending
            .iter()
            .flatten()
            .map(|(_, d)| d.saturating_sub(today))
            .max()
            .unwrap_or(0);
        days.div_ceil(crate::clock::DAYS_PER_MONTH)
    }

    /// Borrow `principal` dollars as a level-payment bond at `bond_rate`
    /// over `bond_term_months()`. Cash arrives now; payments start at the next
    /// month's settlement. Payments falling before the last line now under
    /// construction opens stay out of the coverage test (capitalized
    /// interest, SIM-7). Errors on a non-positive or non-finite amount, above
    /// `bond_capacity()`, or once the authority is bankrupt.
    pub fn issue_bond(&mut self, principal: f64) -> Result<economy::Bond, SimError> {
        if !(principal.is_finite() && principal > 0.0) {
            return Err(SimError(format!(
                "bond principal {principal} must be positive"
            )));
        }
        if let Some(m) = self.ledger.bankrupt_month {
            return Err(SimError(format!("bankrupt since month {m}: no new bonds")));
        }
        let cap = self.bond_capacity();
        if principal > cap * (1.0 + 1e-9) {
            return Err(SimError(format!(
                "bond of ${:.0}M exceeds borrowing capacity ${:.0}M",
                principal / 1e6,
                cap / 1e6
            )));
        }
        let (rate, term) = (self.params.bond_rate, self.params.bond_term_months());
        let grace = self.construction_months_left();
        Ok(self.ledger.issue_bond(principal, rate, term, grace).clone())
    }

    fn line_result(&self, id: LineId) -> Option<&LineResult> {
        self.results.lines.iter().find(|l| l.id == id)
    }

    fn line_capital(&self, id: LineId) -> Option<f64> {
        let l = self.line_result(id)?;
        Some(economy::capital_cost(l, &mode_defaults(l.mode)))
    }

    /// Charge the capital a line costs beyond what was already paid for it,
    /// less any federal grant. Shrinking a line refunds nothing.
    fn charge_capital(&mut self, id: LineId) {
        let Some(total) = self.line_capital(id) else {
            return;
        };
        let boardings = self.line_result(id).map_or(0.0, |l| l.daily_boardings);
        self.charge(id, total, boardings);
    }

    /// Charge `total - already paid` (if positive), less the federal grant
    /// that `boardings` earns.
    fn charge(&mut self, id: LineId, total: f64, boardings: f64) {
        let paid = &mut self.capital_paid[id as usize];
        let spend = (total - *paid).max(0.0);
        *paid = paid.max(total);
        let grant = economy::federal_grant(spend, total, boardings, &self.params);
        self.ledger.charge_capital(spend, grant);
    }

    /// Put lines whose construction finished into service. Returns true if any opened.
    fn open_finished_lines(&mut self) -> bool {
        let today = self.clock.day();
        let mut opened = false;
        for (slot, p) in self.lines.iter_mut().zip(self.pending.iter_mut()) {
            if p.as_ref().is_some_and(|(_, day)| *day <= today) {
                *slot = p.take().map(|(l, _)| l);
                opened = true;
            }
        }
        if opened {
            self.lines_version += 1;
            self.rebuild_transit();
        }
        opened
    }

    fn refresh_pending(&mut self) {
        self.results.pending_lines = self.pending_lines();
    }

    /// The transit tax base: residents of every zone whose centre lies within
    /// `tax_area_km` of an open stop (SIM-9). A Texas MTA taxes whole member
    /// cities, not just the blocks beside a stop; this buffer stands in for
    /// city membership, which grows as service reaches an area.
    fn served_population(&self) -> f64 {
        let stops: Vec<[f64; 2]> = self.net.platforms.iter().map(|s| [s.lon, s.lat]).collect();
        self.taxed_population(&stops)
    }

    /// Residents of zones whose centre is within `tax_area_km` of any of `stops`.
    fn taxed_population(&self, stops: &[[f64; 2]]) -> f64 {
        let r = self.params.tax_area_km;
        self.input
            .zones
            .iter()
            .zip(&self.zpts)
            .filter(|(_, zp)| {
                stops
                    .iter()
                    .any(|s| dist_km(zp.lon, zp.lat, s[0], s[1]) <= r)
            })
            .map(|(z, _)| z.population)
            .sum()
    }

    /// Close the month on the ledger once per month boundary crossed.
    fn settle_months(&mut self) {
        while self.ledger.current.month < self.clock.month() {
            let served = self.served_population();
            self.ledger
                .settle(&self.results.lines, served, &self.params);
        }
    }

    /// Jump the clock to the start of next month, re-run the day and settle the month.
    pub fn skip_to_next_month(&mut self) -> u64 {
        let days = self.clock.skip_to_next_month();
        self.open_finished_lines();
        self.equilibrate_warm();
        self.settle_months();
        days
    }

    /// Replace line `id` (same id). Errors if the id is unknown or removed.
    ///
    /// A service change (headway, fare, speed, span, name, park-and-ride
    /// flags) applies at once. A change of mode or stop locations is new
    /// construction: capital above what the line already paid is charged now,
    /// the old version keeps running, and the new one opens on
    /// `opening_day(mode)` (at once with `instant_build`).
    pub fn update_line(&mut self, id: LineId, line: TransitLine) -> Result<(), SimError> {
        line.validate()?;
        let i = id as usize;
        let drawn = match (self.lines.get(i), self.pending.get(i)) {
            (_, Some(Some((p, _)))) => p,
            (Some(Some(l)), _) => l,
            _ => return Err(SimError(format!("no line {id}"))),
        };
        let rebuild = !same_alignment(drawn, &line);
        if let Some((_, day)) = self.pending[i].take() {
            if !rebuild {
                // Service change to a line with construction under way: the
                // running version (if any) takes the new service at once, the
                // version being built keeps its opening day.
                if let Some(open) = self.lines[i].as_mut() {
                    copy_service(&line, open);
                    self.on_lines_changed();
                    self.charge_capital(id);
                }
                // More vehicles for the version being built cost capital too.
                let (r, capital) = self.evaluate_in_slot(id, &line)?;
                self.charge(id, capital, r.daily_boardings);
                self.pending[i] = Some((line, day));
                self.refresh_pending();
                return Ok(());
            }
        }
        let opens = self.opening_day(line.mode);
        if !rebuild || opens <= self.clock.day() {
            self.lines[i] = Some(line);
            self.on_lines_changed();
            // Charges only positive deltas, so a shorter headway's extra
            // vehicles are paid for and cuts refund nothing (D31).
            self.charge_capital(id);
        } else {
            let (r, capital) = self.evaluate_in_slot(id, &line)?;
            self.charge(id, capital, r.daily_boardings);
            self.pending[i] = Some((line, opens));
        }
        self.refresh_pending();
        Ok(())
    }

    /// Remove line `id` and re-evaluate. The id is retired, never reused.
    pub fn remove_line(&mut self, id: LineId) -> Result<(), SimError> {
        let i = id as usize;
        let open = self.lines.get(i).is_some_and(|l| l.is_some());
        let building = self.pending.get(i).is_some_and(|p| p.is_some());
        if !open && !building {
            return Err(SimError(format!("no line {id}")));
        }
        self.lines[i] = None;
        self.pending[i] = None;
        if open {
            self.on_lines_changed();
        }
        self.refresh_pending();
        Ok(())
    }

    /// Advance the shared clock by `real_dt` real seconds. Each game-day boundary
    /// re-runs the equilibrium, warm-started from the previous day (strategic
    /// tier runs once per game day).
    /// Returns the number of days crossed. Call from a worker thread.
    pub fn tick(&mut self, real_dt: f64) -> u64 {
        let days = self.clock.advance(real_dt);
        if days > 0 {
            self.day_job = None;
            self.open_finished_lines();
            self.equilibrate_warm();
            self.settle_months();
        }
        days
    }

    /// `tick` for a caller that must stay responsive (SIM-18): a day
    /// boundary starts the day's re-run (`equilibrate_warm`'s work) as a job,
    /// and each call runs at most about `budget` of it, so line edits can go
    /// in between calls. Until the job finishes, results and car times are
    /// the previous day's. Lines due to open open at the boundary; months
    /// settle when the job finishes, on that day's results. A boundary that
    /// comes while a job runs is folded into it (the warm start makes days
    /// cumulative). Returns game days crossed. Do not mix with `tick`.
    pub fn tick_budgeted(&mut self, real_dt: f64, budget: std::time::Duration) -> u64 {
        let days = self.clock.advance(real_dt);
        if days > 0 {
            if self.open_finished_lines() {
                // Keep transit consistent with the new network until the job ends.
                self.on_lines_changed();
            }
            if self.day_job.is_none() {
                self.day_job = Some(DayJob::new(self.params.daily_iterations.max(1)));
            }
        }
        if self.day_job.is_some() {
            self.run_day_job(budget);
        }
        days
    }

    /// Share of the running day job done, 0 to 1; None when no job runs.
    pub fn day_progress(&self) -> Option<f64> {
        self.day_job.as_ref().map(|j| j.progress(self.nz))
    }

    /// Run the day job for about `budget` (at least one slice of origins).
    fn run_day_job(&mut self, budget: std::time::Duration) {
        let t0 = Instant::now();
        let Some(mut job) = self.day_job.take() else {
            return;
        };
        let z = self.nz;
        // Origins per slice: enough to keep every core busy, and at least
        // eight slices per pass so a small region still interleaves.
        let slice = (rayon::current_num_threads() * 32)
            .min(z.div_ceil(8))
            .max(1);
        loop {
            match job.round {
                0 => {
                    // Yesterday's lot use sets today's park-and-ride prices (SIM-11).
                    if self.update_pnr_penalties() {
                        self.transit_skims();
                    }
                    job.round = 1;
                }
                r if r <= job.assign_rounds => {
                    let range = job.next..(job.next + slice).min(z);
                    let (vol, _) = self.pass_range(range.clone(), true);
                    if job.vol.is_empty() {
                        job.vol = vol;
                    } else {
                        job.vol.iter_mut().zip(&vol).for_each(|(a, b)| *a += b);
                    }
                    job.next = range.end;
                    if job.next >= z {
                        let aon = std::mem::take(&mut job.vol);
                        self.step_toward(&aon, self.params.feedback_iterations + r);
                        job.round += 1;
                        job.next = 0;
                        job.lines_at_start = self.lines_version;
                    }
                }
                _ => {
                    let range = job.next..(job.next + slice).min(z);
                    let (_, outs) = self.pass_range(range.clone(), false);
                    job.outs.extend(outs);
                    job.next = range.end;
                    if job.next >= z {
                        // A line edit during this pass left some transit rows
                        // stale: keep the car skims, redo transit at once.
                        let fresh = job.lines_at_start == self.lines_version;
                        let rows = self.commit_outs(std::mem::take(&mut job.outs), fresh);
                        if fresh {
                            self.finish(t0, false, Some(rows));
                        } else {
                            self.transit_skims();
                            self.finish(t0, false, None);
                        }
                        self.settle_months();
                        return;
                    }
                }
            }
            if t0.elapsed() >= budget {
                break;
            }
        }
        self.day_job = Some(job);
    }

    /// Full run from free-flow: mode choice <-> congested car assignment to a
    /// stable state, then transit loading. `Sim::new` calls it; call it again
    /// after changing `params`. Costs `feedback_iterations + 1` shortest-path
    /// trees per zone.
    pub fn equilibrate(&mut self) {
        let t0 = Instant::now();
        self.day_job = None;
        self.link_time = self.graph.free_flow_times();
        self.link_vol.iter_mut().for_each(|v| *v = 0.0);
        let k = self.params.feedback_iterations.max(1);
        self.feedback(k, 0);
        let (rows, _) = self.pass(false);
        self.finish(t0, false, Some(rows));
        self.fit_park_and_ride(t0);
    }

    /// Full lots: re-price, re-skim and re-choose until park-and-ride fits
    /// the spaces (SIM-11). Road volumes stay as they are; only the transit
    /// skims and mode choice move. No-op when no lot has a space limit.
    fn fit_park_and_ride(&mut self, t0: Instant) {
        for _ in 0..self.params.pnr_iterations {
            if !self.update_pnr_penalties() {
                break;
            }
            self.transit_skims();
            // Lot overflow is a small share of trips: keep the stale flag.
            self.finish(t0, self.results.car_times_stale, None);
        }
    }

    /// One shadow-price step for lots with a space limit: raise the access
    /// penalty where parked cars exceed spaces, relax it where they fall
    /// short. Returns true if any lot is over by more than 5% (worth another
    /// pass) or a penalty changed materially.
    fn update_pnr_penalties(&mut self) -> bool {
        let mut again = false;
        let step = self.params.pnr_penalty_step_min;
        for (pi, pl) in self.net.platforms.iter().enumerate() {
            if !pl.park_ride || !pl.pnr_spaces.is_finite() {
                continue;
            }
            let cars = self.pnr_cars.get(pi).copied().unwrap_or(0.0);
            let ratio = cars / pl.pnr_spaces.max(1.0);
            let old = self.net.pnr_penalty[pi];
            // Log step with the ratio clamped to [0.2, 5]: each round moves
            // the price at most step x 1.6 either way, so a lot 30x over on
            // the first pass does not price itself empty for days.
            let new = (old + step * ratio.clamp(0.2, 5.0).ln()).max(0.0);
            if ratio > 1.05 || (new - old).abs() > 0.5 {
                again = true;
            }
            self.net.pnr_penalty[pi] = new;
            let line = &self.net.lines[pl.line as usize];
            self.pnr_penalty
                .insert((line.id, pi - line.first_platform as usize), new);
        }
        again
    }

    /// Daily re-run, warm-started from the previous day's link volumes, so
    /// `daily_iterations` rounds suffice. `tick` calls it at day boundaries.
    pub fn equilibrate_warm(&mut self) {
        let t0 = Instant::now();
        self.day_job = None;
        // The previous solution counts as `feedback_iterations` rounds of
        // history: that sets the floor under each day's line-searched step.
        let prior = self.params.feedback_iterations;
        // Yesterday's lot use sets today's park-and-ride prices (SIM-11).
        if self.update_pnr_penalties() {
            self.transit_skims();
        }
        self.feedback(self.params.daily_iterations.max(1), prior);
        let (rows, _) = self.pass(false);
        self.finish(t0, false, Some(rows));
    }

    /// `rounds` of: one fused pass (skims, mode choice, all-or-nothing load on
    /// the same trees), a step toward that load, BPR times. The first round
    /// from empty roads takes the load whole; every later step is the
    /// Frank-Wolfe line search ([`Sim::fw_step`], SIM-15), floored at a tenth
    /// of the MSA step 1/(prior + n) so a round that finds nothing better
    /// still moves.
    fn feedback(&mut self, rounds: usize, prior: usize) {
        for n in 1..=rounds {
            let (_, aon) = self.pass(true);
            self.step_toward(&aon, prior + n);
        }
    }

    /// Move link volumes toward the all-or-nothing load `aon` as round `n`
    /// of history (the whole load when `n` is 1), then update BPR times.
    fn step_toward(&mut self, aon: &[f64], n: usize) {
        let msa = 1.0 / n as f64;
        let step = if msa >= 1.0 {
            1.0
        } else {
            self.fw_step(aon).max(0.1 * msa)
        };
        for (v, a) in self.link_vol.iter_mut().zip(aon) {
            *v += step * (a - *v);
        }
        self.update_link_times();
    }

    /// Frank-Wolfe step: the `λ` in [0, 1] that minimizes the Beckmann
    /// objective along `link_vol -> aon`, by bisection on its derivative
    /// `Σ (a - v) t(v + λ (a - v))`. MSA's fixed 1/n steps leave a quarter
    /// of the free-flow load on the fastest roads after four rounds, which on
    /// the highway-only statewide graph meant freeways at 3x capacity and
    /// 3 km/h peak speeds; the line search spreads it in the same rounds.
    fn fw_step(&self, aon: &[f64]) -> f64 {
        let (a, b) = (self.params.bpr_alpha, self.params.bpr_beta);
        let g = &self.graph;
        let slope = |l: f64| -> f64 {
            (0..g.edges.len())
                .into_par_iter()
                .with_min_len(4096)
                .map(|e| {
                    let d = aon[e] - self.link_vol[e];
                    if d == 0.0 {
                        0.0
                    } else {
                        d * g.bpr_time(e, self.link_vol[e] + l * d, a, b)
                    }
                })
                .sum()
        };
        if slope(1.0) <= 0.0 {
            return 1.0;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..20 {
            let mid = 0.5 * (lo + hi);
            if slope(mid) > 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// Re-fit the alternative-specific constants so **commute** (home-work)
    /// trips reproduce `targets`, e.g. ACS journey-to-work shares (any scale;
    /// normalized). Non-work trips use the same constants. If
    /// `targets.transit_drive` is 0, `transit_walk` is the whole transit
    /// share and both access constants move together (ACS does not split
    /// them); otherwise each access mode is fitted to its own target. Choices with a zero
    /// target or no modeled trips are left alone. Each iteration re-runs
    /// mode choice on current skims (cheap); call `equilibrate_warm` between
    /// rounds to let congestion respond.
    pub fn calibrate_mode_shares(&mut self, targets: ModeSplit, iterations: usize) {
        let tgt = targets.shares().to_array();
        for _ in 0..iterations {
            let rows = self.choose_all();
            let mut tot = [0.0; N_CHOICES];
            for r in &rows {
                for (m, t) in tot.iter_mut().enumerate() {
                    *t += r.work[m];
                }
            }
            let sum: f64 = tot.iter().sum();
            if sum <= 0.0 || tot[DRIVE] <= 0.0 {
                break;
            }
            let base = (tgt[DRIVE] / (tot[DRIVE] / sum)).ln();
            let step = |t: f64, m: f64| (t / (m / sum)).ln() - base;
            for m in [CARPOOL, WALK, BIKE] {
                if tgt[m] > 0.0 && tot[m] > 0.0 {
                    self.params.asc[m] += step(tgt[m], tot[m]);
                }
            }
            if tgt[TRANSIT_DRIVE] > 0.0 {
                // Both transit access modes have their own target.
                for m in [TRANSIT_WALK, TRANSIT_DRIVE] {
                    if tgt[m] > 0.0 && tot[m] > 0.0 {
                        self.params.asc[m] += step(tgt[m], tot[m]);
                    }
                }
            } else {
                let (tt, mt) = (tgt[TRANSIT_WALK], tot[TRANSIT_WALK] + tot[TRANSIT_DRIVE]);
                if tt > 0.0 && mt > 0.0 {
                    let d = step(tt, mt);
                    self.params.asc[TRANSIT_WALK] += d;
                    self.params.asc[TRANSIT_DRIVE] += d;
                }
            }
        }
        self.finish(Instant::now(), self.results.car_times_stale, None);
    }

    /// Kept zone pairs with no free-flow car time although the reverse
    /// direction has one: a diagnostic for the strategic cutoff (0 when the
    /// trees reach far enough for asymmetric road times).
    pub fn car_unreachable_pairs(&self) -> usize {
        let t = &self.car_free.time;
        (0..self.pairs.len())
            .filter(|&k| !t[k].is_finite() && t[self.pairs.rev[k]].is_finite())
            .count()
    }

    /// Kept zone pairs whose congested car time hit the peak trees' guard
    /// (4x the strategic cutoff, SIM-14): they keep the car option at the
    /// guard time, but their car trips are not loaded on the roads (SIM-15).
    /// 0 without a cutoff.
    pub fn car_past_guard_pairs(&self) -> usize {
        let Some(cut) = self.params.strategic_max_min else {
            return 0;
        };
        let guard = (cut * PEAK_GUARD) as f32;
        self.car_peak.time.iter().filter(|&&t| t >= guard).count()
    }

    /// Commute (home-work) mode shares under the current constants and skims,
    /// the quantity `calibrate_mode_shares` fits.
    pub fn commute_mode_shares(&self) -> ModeSplit {
        let mut tot = [0.0; N_CHOICES];
        for r in &self.choose_all() {
            for (m, t) in tot.iter_mut().enumerate() {
                *t += r.work[m];
            }
        }
        ModeSplit::from_array(tot).shares()
    }

    /// Per-zone trips, transit trips and stop coverage.
    pub fn zone_results(&self) -> Vec<ZoneResult> {
        (0..self.nz)
            .map(|i| {
                let tr: f64 = self.transit_od[self.pairs.row(i)].iter().sum();
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

    /// Peak-hour flows on every directed traversal of every input road link
    /// (a contracted chain reports each of its links; pruned dead ends report
    /// zero volume at free-flow speed).
    pub fn link_flows(&self) -> Vec<LinkFlow> {
        let (a, b) = (self.params.bpr_alpha, self.params.bpr_beta);
        let node = |v: u32| self.input.road_nodes[v as usize].id;
        let flow = |m: &crate::graph::Member, vol: f64| {
            let t = m.t0_min * (1.0 + a * (vol / m.cap_vph).powf(b));
            LinkFlow {
                link: m.link,
                from: node(m.from),
                to: node(m.to),
                volume_vph: vol,
                vc: vol / m.cap_vph,
                speed_kph: m.len_km / (t / 60.0),
            }
        };
        let mut out: Vec<LinkFlow> = (0..self.graph.edges.len())
            .flat_map(|e| {
                let vol = self.link_vol[e];
                self.graph.members_of(e).iter().map(move |m| flow(m, vol))
            })
            .collect();
        out.extend(self.graph.dead.iter().map(|m| flow(m, 0.0)));
        out
    }

    /// Top `n` zone pairs by daily trips (both directions, all modes, excluding intrazonal).
    pub fn desire_lines(&self, n: usize) -> Vec<DesireLine> {
        let mut out = Vec::new();
        for i in 0..self.nz {
            for a in self.pairs.row(i) {
                let j = self.pairs.dest[a] as usize;
                if j <= i {
                    continue;
                }
                let b = self.pairs.rev[a];
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
        out.sort_by(|a, b| b.trips.total_cmp(&a.trips));
        out.truncate(n);
        out
    }

    // ----------------------------------------------------------- internals

    /// Re-evaluate transit after a line change. Returns the state it
    /// replaced, which `restore_transit` puts back; callers that keep the
    /// change just drop it. Everything is moved, not copied, except the
    /// results and park-and-ride prices, which are small.
    fn on_lines_changed(&mut self) -> TransitState {
        let t0 = Instant::now();
        self.lines_version += 1;
        let saved = TransitState {
            net: self.rebuild_transit(),
            tsk: [
                std::mem::take(&mut self.tsk_walk),
                std::mem::take(&mut self.tsk_drive),
                std::mem::take(&mut self.tsk_walk_off),
                std::mem::take(&mut self.tsk_drive_off),
            ],
            transit_od: std::mem::take(&mut self.transit_od),
            zone_trips: std::mem::take(&mut self.zone_trips),
            results: self.results.clone(),
            pnr_cars: std::mem::take(&mut self.pnr_cars),
            pnr_penalty: self.pnr_penalty.clone(),
            loads: std::mem::take(&mut self.loads),
        };
        match self.edit_scope(&saved) {
            Some(scope) => {
                self.transit_skims_scoped(&scope, &saved.tsk);
                self.finish_scoped(t0, true, None, Some((&scope, &saved.loads)));
            }
            None => {
                self.transit_skims();
                self.finish(t0, true, None);
            }
        }
        self.fit_park_and_ride(t0);
        saved
    }

    /// Put back the transit state `on_lines_changed` replaced.
    fn restore_transit(&mut self, s: TransitState) {
        self.net = s.net;
        [
            self.tsk_walk,
            self.tsk_drive,
            self.tsk_walk_off,
            self.tsk_drive_off,
        ] = s.tsk;
        self.transit_od = s.transit_od;
        self.zone_trips = s.zone_trips;
        self.results = s.results;
        self.pnr_cars = s.pnr_cars;
        self.pnr_penalty = s.pnr_penalty;
        self.loads = s.loads;
    }

    /// Which origins and lines the edit from `old` to `self.net` can change,
    /// or None when it can change all of them (off-peak skims switch on or
    /// off, or the old state is incomplete).
    fn edit_scope(&self, old: &TransitState) -> Option<EditScope> {
        let (on, nn) = (&old.net, &self.net);
        let z = self.nz;
        if on.offpeak_differs() != nn.offpeak_differs()
            || old.tsk[0].len() != z
            || old.tsk[1].len() != z
            || old.loads.boardings.len() != on.lines.len()
        {
            return None;
        }
        let (so, sn) = (on.comp_sigs(), nn.comp_sigs());
        let (set_o, set_n): (HashSet<u64>, HashSet<u64>) =
            (so.iter().copied().collect(), sn.iter().copied().collect());
        let gone: Vec<bool> = so.iter().map(|h| !set_n.contains(h)).collect();
        let fresh: Vec<bool> = sn.iter().map(|h| !set_o.contains(h)).collect();
        let search: Vec<bool> = (0..z)
            .into_par_iter()
            .map(|i| {
                self.seed_comps(nn, i).any(|c| fresh[c as usize])
                    || self.seed_comps(on, i).any(|c| gone[c as usize])
            })
            .collect();
        // Components a re-searched origin reaches: their lines' loads change.
        let mut touched = fresh.clone();
        for i in (0..z).filter(|&i| search[i]) {
            for c in self.seed_comps(nn, i) {
                touched[c as usize] = true;
            }
        }
        let load = (0..z)
            .into_par_iter()
            .map(|i| self.seed_comps(nn, i).any(|c| touched[c as usize]))
            .collect();
        let old_line: HashMap<u32, usize> = on
            .lines
            .iter()
            .enumerate()
            .map(|(k, l)| (l.id, k))
            .collect();
        let keep = nn
            .lines
            .iter()
            .map(|l| {
                let c = nn.comp[l.first_platform as usize] as usize;
                if touched[c] {
                    None
                } else {
                    old_line.get(&l.id).copied()
                }
            })
            .collect();
        let new_plat: HashMap<(u32, u32), u32> = nn
            .platforms
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let l = &nn.lines[p.line as usize];
                ((l.id, k as u32 - l.first_platform), k as u32)
            })
            .collect();
        let plat_map = on
            .platforms
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let l = &on.lines[p.line as usize];
                new_plat
                    .get(&(l.id, k as u32 - l.first_platform))
                    .copied()
                    .unwrap_or(NONE)
            })
            .collect();
        Some(EditScope {
            search,
            load,
            keep,
            plat_map,
        })
    }

    /// Components origin `i`'s searches on `net` can seed: those of its walk
    /// access platforms and of the park-and-ride platforms within the drive
    /// limit (the same test `Searcher::run` applies).
    fn seed_comps<'a>(&'a self, net: &'a TransitNet, i: usize) -> impl Iterator<Item = u32> + 'a {
        let walk = net.walk_access[i]
            .iter()
            .map(|&(p, _)| net.comp[p as usize]);
        let p = &self.params;
        let drive = net.pnr.iter().filter_map(move |&pl| {
            let zn = net.platforms[pl as usize].zone_near as usize;
            let k = self.pairs.find(i, zn)?;
            let dm = self.car_peak.time[k] as f64 + p.pnr_park_min;
            (dm.is_finite() && dm <= p.pnr_max_drive_min).then_some(net.comp[pl as usize])
        });
        walk.chain(drive)
    }

    /// Rebuild the transit network from the active lines; returns the old one.
    fn rebuild_transit(&mut self) -> TransitNet {
        let active: Vec<(u32, &TransitLine)> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|l| (i as u32, l)))
            .collect();
        let (router, graph) = (&mut self.router, &self.graph);
        let net = TransitNet::build(&active, &self.zpts, &self.params, &mut |a, b| {
            router.route(graph, a, b)
        });
        let old = std::mem::replace(&mut self.net, net);
        for (pi, pl) in self.net.platforms.iter().enumerate() {
            let line = &self.net.lines[pl.line as usize];
            let key = (line.id, pi - line.first_platform as usize);
            self.net.pnr_penalty[pi] = self.pnr_penalty.get(&key).copied().unwrap_or(0.0);
        }
        old
    }

    /// Mode choice on current skims, transit loading, and results.
    fn finish(&mut self, t0: Instant, stale: bool, rows: Option<Vec<Row>>) {
        self.finish_scoped(t0, stale, rows, None);
    }

    /// `finish`; with a scope, only its origins are loaded and the other
    /// lines keep their old loads (SIM-21).
    fn finish_scoped(
        &mut self,
        t0: Instant,
        stale: bool,
        rows: Option<Vec<Row>>,
        scope: Option<(&EditScope, &LineLoads)>,
    ) {
        let idle = self.transit_idle();
        let rows = match rows {
            Some(r) => r,
            None => self.choose_all_reusing(&idle),
        };
        self.store_idle(&rows, &idle);
        let np = self.pairs.len();
        let wt_work = self.flatten(&rows, |r| &r.wt_work);
        let dt_work = self.flatten(&rows, |r| &r.dt_work);
        let wt_other = self.flatten(&rows, |r| &r.wt_other);
        let dt_other = self.flatten(&rows, |r| &r.dt_other);
        // Daily transit trips: AM commute i->j, PM return j->i, plus other trips.
        // Park-and-ride returns are loaded on the AM path (see MODEL.md).
        // Work trips ride peak paths; other trips ride off-peak paths, which
        // differ only when some line has its own off-peak headway.
        let off = !self.net.is_empty() && self.net.offpeak_differs();
        let mut d_walk = vec![0.0; np];
        let mut d_drive = vec![0.0; np];
        let (mut o_walk, mut o_drive) = if off {
            (vec![0.0; np], vec![0.0; np])
        } else {
            (vec![], vec![])
        };
        for k in 0..np {
            d_walk[k] = wt_work[k] + wt_work[self.pairs.rev[k]];
            d_drive[k] = 2.0 * dt_work[k];
            if off {
                o_walk[k] = wt_other[k];
                o_drive[k] = dt_other[k];
            } else {
                d_walk[k] += wt_other[k];
                d_drive[k] += dt_other[k];
            }
        }
        self.pnr_cars = vec![0.0; self.net.platforms.len()];
        if !self.net.pnr.is_empty() {
            let tsk_other = if off {
                &self.tsk_drive_off
            } else {
                &self.tsk_drive
            };
            for (i, (peak, other)) in self.tsk_drive.iter().zip(tsk_other).enumerate() {
                let r = self.pairs.row(i);
                for (q, w) in peak.iter().enumerate() {
                    let k = r.start + q;
                    if w.pnr_plat != NONE && dt_work[k] > 0.0 {
                        self.pnr_cars[w.pnr_plat as usize] += dt_work[k];
                    }
                }
                for (q, o) in other.iter().enumerate() {
                    let k = r.start + q;
                    if o.pnr_plat != NONE && dt_other[k] > 0.0 {
                        // A round trip of two one-way other trips parks one car.
                        self.pnr_cars[o.pnr_plat as usize] += 0.5 * dt_other[k];
                    }
                }
            }
        }
        let only = scope.map(|(s, _)| &s.load[..]);
        let mut loads = self.load_transit(&d_walk, &d_drive, false, only);
        if off {
            loads = loads.merge(self.load_transit(&o_walk, &o_drive, true, only));
        }
        if let Some((s, old)) = scope {
            loads.carry(old, &s.keep);
        }
        // Built fresh (not written in place) so `evaluate_in_slot` can move
        // the previous vector aside and back.
        self.transit_od = (0..np)
            .map(|k| d_walk[k] + d_drive[k] + if off { o_walk[k] + o_drive[k] } else { 0.0 })
            .collect();

        let mut work = [0.0; N_CHOICES];
        let mut other = [0.0; N_CHOICES];
        let mut unserved = 0.0;
        let mut zone_trips = Vec::with_capacity(rows.len());
        for r in &rows {
            let mut zt = 0.0;
            for m in 0..N_CHOICES {
                work[m] += 2.0 * r.work[m];
                other[m] += r.other[m];
                zt += 2.0 * r.work[m] + r.other[m];
            }
            zone_trips.push(zt);
            unserved += r.unserved;
        }
        self.zone_trips = zone_trips;
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
        self.loads = loads;

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
            beyond_cutoff_trips: self.beyond_cutoff.iter().sum(),
            lines,
            peak_vehicle_km: vkm,
            peak_avg_speed_kph: if vhr > 0.0 { vkm / vhr } else { 0.0 },
            work_peak_hour_share: wph,
            car_times_stale: stale,
            eval_ms: t0.elapsed().as_secs_f64() * 1000.0,
            pending_lines: self.pending_lines(),
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
                let peak_vehicles = (cycle / l.headway).ceil().max(1.0);
                let off_vehicles = (cycle / l.offpeak_headway).ceil().max(1.0);
                // The fleet covers whichever period runs more often, so an
                // off-peak headway shorter than the peak one is paid for too.
                let vehicles = peak_vehicles.max(off_vehicles);
                let peak_h = self.params.peak_service_hours.clamp(0.0, l.span_hours);
                let rev_hours = peak_vehicles * peak_h + off_vehicles * (l.span_hours - peak_h);
                let op_cost = rev_hours * l.defaults.cost_per_revenue_hour();
                let revenue = loads.boardings[li] * l.fare + loads.pkm[li] * l.fare_per_km;
                LineResult {
                    id: l.id,
                    name: l.name.clone(),
                    mode: l.mode,
                    length_km: l.length_km(),
                    run_min: l.run_min(),
                    daily_boardings: loads.boardings[li],
                    boardings_by_stop: loads.stop_boardings[li].clone(),
                    park_ride_cars_by_stop: (0..loads.stop_boardings[li].len())
                        .map(|i| {
                            let pi = l.first_platform as usize + i;
                            self.pnr_cars.get(pi).copied().unwrap_or(0.0)
                        })
                        .collect(),
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
                    segment_shapes: l.seg_shape.clone(),
                }
            })
            .collect()
    }

    /// Per-pair vector over all rows; a reused idle row contributes zeros.
    fn flatten(&self, rows: &[Row], f: impl Fn(&Row) -> &Vec<f64>) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.pairs.len());
        for (i, r) in rows.iter().enumerate() {
            let v = f(r);
            if v.is_empty() {
                out.resize(out.len() + self.pairs.row(i).len(), 0.0);
            } else {
                out.extend_from_slice(v);
            }
        }
        out
    }

    /// Origins with no transit option to any kept destination, peak or
    /// off-peak, walk or drive access.
    fn transit_idle(&self) -> Vec<bool> {
        let none = |v: &TSkim, i: usize| v.is_empty() || v[i].is_empty();
        (0..self.nz)
            .into_par_iter()
            .map(|i| {
                none(&self.tsk_walk, i)
                    && none(&self.tsk_drive, i)
                    && none(&self.tsk_walk_off, i)
                    && none(&self.tsk_drive_off, i)
            })
            .collect()
    }

    fn params_key(&self) -> Option<String> {
        serde_json::to_string(&self.params).ok()
    }

    /// `choose_all`, but origins with no transit reuse their stored totals
    /// when car times and params have not changed since they were stored.
    /// On a statewide build most tracts have no transit, so a line edit
    /// re-chooses only the few that do.
    fn choose_all_reusing(&self, idle: &[bool]) -> Vec<Row> {
        let valid = self.idle_key.is_some() && self.idle_key == self.params_key();
        (0..self.nz)
            .into_par_iter()
            .map(|i| match (valid && idle[i], self.idle_rows.get(i)) {
                (true, Some(Some((work, other, unserved)))) => Row {
                    car_peak: vec![],
                    wt_work: vec![],
                    dt_work: vec![],
                    wt_other: vec![],
                    dt_other: vec![],
                    work: *work,
                    other: *other,
                    unserved: *unserved,
                },
                _ => self.choose_row(i, &self.row_inputs(i)),
            })
            .collect()
    }

    /// Store the totals of idle origins chosen in full; start a new store
    /// if params or car times changed.
    fn store_idle(&mut self, rows: &[Row], idle: &[bool]) {
        let key = self.params_key();
        if self.idle_key.is_none() || self.idle_key != key {
            self.idle_rows = vec![None; self.nz];
            self.idle_key = key;
        }
        for (i, r) in rows.iter().enumerate() {
            if !idle[i] {
                self.idle_rows[i] = None;
            } else if !self.pairs.row(i).is_empty() && r.wt_work.len() == self.pairs.row(i).len() {
                self.idle_rows[i] = Some((r.work, r.other, r.unserved));
            }
        }
    }

    fn crow_km(&self, i: usize, j: usize) -> f64 {
        if i == j {
            return self.intra_km[i];
        }
        let (a, b) = (&self.zpts[i], &self.zpts[j]);
        dist_km(a.lon, a.lat, b.lon, b.lat)
    }

    /// A shortest-path tree buffer, bounded at the strategic cutoff
    /// (`slack` x cutoff, so congested times above it still reach their pairs).
    fn tree(&self, slack: f64) -> Tree {
        let mut t = Tree::new(self.graph.n_nodes, &self.zone_node);
        if let Some(cut) = self.params.strategic_max_min {
            t.bound = cut * slack;
        }
        t
    }

    /// Shortest-path tree from zone `i`, run until every kept destination is
    /// settled (or past the tree's bound). With a cutoff the targets are the
    /// row's own zones, so asymmetric road times cannot leave a kept pair
    /// without a car time.
    fn run_tree(&self, tree: &mut Tree, times: &[f64], i: usize) {
        if self.params.strategic_max_min.is_some() {
            tree.set_targets(
                self.pairs
                    .dests(i)
                    .iter()
                    .map(|&j| self.zone_node[j as usize]),
            );
        }
        tree.run(&self.graph, times, self.zone_node[i]);
    }

    /// Car skims for every kept pair on `times` (used once, for free-flow times).
    fn skim_car(&self, times: &[f64]) -> CarSkim {
        let z = self.nz;
        let np = self.pairs.len();
        let g = &self.graph;
        let rows: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = (0..z)
            .into_par_iter()
            .map_init(
                // Kept pairs are within the cutoff both ways at free flow, so
                // the bound only guards against float ties.
                || (self.tree(1.01), vec![0.0; g.n_nodes], vec![0.0; g.n_nodes]),
                |(tree, km, toll), i| {
                    self.run_tree(tree, times, i);
                    tree.km_and_toll(g, km, toll);
                    self.car_row(i, tree, km, toll)
                },
            )
            .collect();
        let mut s = CarSkim {
            time: Vec::with_capacity(np),
            km: Vec::with_capacity(np),
            toll: Vec::with_capacity(np),
        };
        for (t, k, tl) in rows {
            s.time.extend(t);
            s.km.extend(k);
            s.toll.extend(tl);
        }
        s
    }

    fn update_link_times(&mut self) {
        let (a, b) = (self.params.bpr_alpha, self.params.bpr_beta);
        for e in 0..self.graph.edges.len() {
            self.link_time[e] = self.graph.bpr_time(e, self.link_vol[e], a, b);
        }
    }

    /// Singly constrained gravity model for non-work trips on free-flow car time.
    fn gravity_other(&self) -> Vec<f64> {
        let z = self.nz;
        let zones = &self.input.zones;
        let p = &self.params;
        let rows: Vec<Vec<f64>> = (0..z)
            .into_par_iter()
            .map(|i| {
                // Total trips minus this zone's work trips (both directions).
                // Work trips beyond the cutoff still count against the budget.
                let work: f64 =
                    self.hbw[self.pairs.row(i)].iter().sum::<f64>() * 2.0 + self.beyond_cutoff[i];
                let prod = (zones[i].population * p.trips_per_person - work).max(0.0);
                let mut w: Vec<f64> = self
                    .pairs
                    .row(i)
                    .map(|k| {
                        let j = self.pairs.dest[k] as usize;
                        let t = self.car_free.time[k] as f64;
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

    /// `transit_skims` for the origins `scope` marks; the others keep their
    /// rows from `old` (park-and-ride ids renumbered).
    fn transit_skims_scoped(&mut self, scope: &EditScope, old: &[TSkim; 4]) {
        let z = self.nz;
        let (net, p, car, pairs) = (&self.net, &self.params, &self.car_peak, &self.pairs);
        let off = net.offpeak_differs();
        let identity = scope
            .plat_map
            .iter()
            .enumerate()
            .all(|(k, &v)| v == NONE || v as usize == k);
        let keep_row = |v: &TSkim, i: usize| -> Vec<TOd> {
            let mut row = v.get(i).cloned().unwrap_or_default();
            if !identity {
                for o in &mut row {
                    if o.pnr_plat != NONE {
                        o.pnr_plat = scope.plat_map[o.pnr_plat as usize];
                    }
                }
            }
            row
        };
        let rows: Vec<[Vec<TOd>; 4]> = (0..z)
            .into_par_iter()
            .map_init(
                || Searcher::new(net, p, z),
                |s, i| {
                    if !scope.search[i] {
                        return std::array::from_fn(|k| keep_row(&old[k], i));
                    }
                    let r = pairs.row(i);
                    let (cm, ck) = dense_row(pairs.dests(i), &car.time[r.clone()], &car.km[r], z);
                    let (w, d) = transit_row(s, i, &cm, &ck, false, pairs.dests(i));
                    let (wo, dofp) = if off {
                        transit_row(s, i, &cm, &ck, true, pairs.dests(i))
                    } else {
                        (vec![], vec![])
                    };
                    [w, d, wo, dofp]
                },
            )
            .collect();
        self.store_tsk(rows, off);
    }

    fn store_tsk(&mut self, rows: Vec<[Vec<TOd>; 4]>, off: bool) {
        self.tsk_walk.clear();
        self.tsk_drive.clear();
        self.tsk_walk_off.clear();
        self.tsk_drive_off.clear();
        for [w, d, wo, dofp] in rows {
            self.tsk_walk.push(w);
            self.tsk_drive.push(d);
            if off {
                self.tsk_walk_off.push(wo);
                self.tsk_drive_off.push(dofp);
            }
        }
    }

    fn transit_skims(&mut self) {
        let z = self.nz;
        self.tsk_walk_off.clear();
        self.tsk_drive_off.clear();
        if self.net.is_empty() {
            self.tsk_walk = vec![vec![]; z];
            self.tsk_drive = vec![vec![]; z];
            return;
        }
        let (net, p, car, pairs) = (&self.net, &self.params, &self.car_peak, &self.pairs);
        let off = net.offpeak_differs();
        let rows: Vec<[Vec<TOd>; 4]> = (0..z)
            .into_par_iter()
            .map_init(
                || Searcher::new(net, p, z),
                |s, i| {
                    let r = pairs.row(i);
                    let (cm, ck) = dense_row(pairs.dests(i), &car.time[r.clone()], &car.km[r], z);
                    let (w, d) = transit_row(s, i, &cm, &ck, false, pairs.dests(i));
                    let (wo, dofp) = if off {
                        transit_row(s, i, &cm, &ck, true, pairs.dests(i))
                    } else {
                        (vec![], vec![])
                    };
                    [w, d, wo, dofp]
                },
            )
            .collect();
        self.store_tsk(rows, off);
    }

    /// Load daily transit demand on peak (`offpeak = false`) or off-peak paths.
    /// Only origins `only` marks are loaded (all when None).
    fn load_transit(
        &self,
        d_walk: &[f64],
        d_drive: &[f64],
        offpeak: bool,
        only: Option<&[bool]>,
    ) -> LineLoads {
        let z = self.nz;
        let net = &self.net;
        if net.is_empty() {
            return LineLoads::new(net);
        }
        let (p, car, pairs) = (&self.params, &self.car_peak, &self.pairs);
        (0..z)
            .into_par_iter()
            .fold(
                || {
                    let mut s = Searcher::new(net, p, z);
                    s.offpeak = offpeak;
                    (s, LineLoads::new(net))
                },
                |(mut s, mut acc), i| {
                    if only.is_some_and(|o| !o[i]) {
                        return (s, acc);
                    }
                    let r = pairs.row(i);
                    let dests = pairs.dests(i);
                    let mut car_rows = None;
                    for (access, d) in [(Access::Walk, d_walk), (Access::Drive, d_drive)] {
                        let row = &d[r.clone()];
                        if row.iter().any(|&x| x > 0.0) {
                            let (cm, ck) = car_rows.get_or_insert_with(|| {
                                dense_row(dests, &car.time[r.clone()], &car.km[r.clone()], z)
                            });
                            if !s.run(i, access, cm, ck) {
                                continue;
                            }
                            for (&j, &x) in dests.iter().zip(row) {
                                s.load(j as usize, x, &mut acc);
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
        let z = self.nz;
        (0..z)
            .into_par_iter()
            .map(|i| self.choose_row(i, &self.row_inputs(i)))
            .collect()
    }

    /// Origin `i`'s transit row from `v` (or from `peak` when `v` is an
    /// empty off-peak skim), padded with `TOd::NONE` when it has none.
    fn trow<'a>(&'a self, v: &'a TSkim, peak: &'a TSkim, i: usize) -> &'a [TOd] {
        let row = if v.is_empty() { &peak[i] } else { &v[i] };
        if row.is_empty() {
            &self.none_row[..self.pairs.row(i).len()]
        } else {
            row
        }
    }

    /// Current skims for origin `i`'s row.
    fn row_inputs(&self, i: usize) -> RowInputs<'_> {
        let r = self.pairs.row(i);
        RowInputs {
            peak_t: &self.car_peak.time[r.clone()],
            peak_km: &self.car_peak.km[r.clone()],
            peak_toll: &self.car_peak.toll[r.clone()],
            tw: self.trow(&self.tsk_walk, &self.tsk_walk, i),
            td: self.trow(&self.tsk_drive, &self.tsk_drive, i),
            tw_off: self.trow(&self.tsk_walk_off, &self.tsk_walk, i),
            td_off: self.trow(&self.tsk_drive_off, &self.tsk_drive, i),
        }
    }

    /// One parallel pass over origins: shortest-path tree on current link
    /// times, car skim row, transit skim row, mode choice row and (if `vol`)
    /// all-or-nothing loading of that row's peak car demand on the same tree.
    /// Updates `car_peak`, `tsk_walk`, `tsk_drive`. One tree per origin
    /// instead of two (skim, then assign).
    fn pass(&mut self, assign: bool) -> (Vec<Row>, Vec<f64>) {
        let (vol, outs) = self.pass_range(0..self.nz, assign);
        (self.commit_outs(outs, true), vol)
    }

    /// The fused pass for origins `origins` only: their all-or-nothing road
    /// volumes (empty unless `assign`) and outputs. Reads state, changes
    /// nothing, so a game day can run it a slice at a time (SIM-18).
    fn pass_range(
        &self,
        origins: std::ops::Range<usize>,
        assign: bool,
    ) -> (Vec<f64>, Vec<(usize, OriginOut)>) {
        let z = self.nz;
        let g = &self.graph;
        let ne = if assign { g.edges.len() } else { 0 };
        let times = &self.link_time;
        let (net, p) = (&self.net, &self.params);
        let has_transit = !net.is_empty();
        let off = has_transit && net.offpeak_differs();
        type Acc<'a> = (
            Tree,
            Vec<f64>,
            Vec<f64>,
            Searcher<'a>,
            Vec<f64>,
            Vec<(usize, OriginOut)>,
        );
        origins
            .into_par_iter()
            .fold(
                || -> (Vec<f64>, Option<Acc>) { (vec![0.0; ne], None) },
                |(mut vol, acc), i| {
                    let (mut tree, mut km, mut toll, mut srch, mut nd, mut outs) = acc
                        .unwrap_or_else(|| {
                            (
                                self.tree(PEAK_GUARD),
                                vec![0.0; g.n_nodes],
                                vec![0.0; g.n_nodes],
                                Searcher::new(net, p, z),
                                vec![0.0; g.n_nodes],
                                Vec::new(),
                            )
                        });
                    self.run_tree(&mut tree, times, i);
                    tree.km_and_toll(g, &mut km, &mut toll);
                    let (t, k, tl) = self.car_row(i, &tree, &km, &toll);
                    let dests = self.pairs.dests(i);
                    let dense = if has_transit {
                        Some(dense_row(dests, &t, &k, z))
                    } else {
                        None
                    };
                    let (w, d) = match &dense {
                        Some((cm, ck)) => transit_row(&mut srch, i, cm, ck, false, dests),
                        None => (vec![], vec![]),
                    };
                    let (w_off, d_off) = if let (true, Some((cm, ck))) = (off, &dense) {
                        transit_row(&mut srch, i, cm, ck, true, dests)
                    } else {
                        (vec![], vec![])
                    };
                    let none = &self.none_row[..dests.len()];
                    let inp = RowInputs {
                        peak_t: &t,
                        peak_km: &k,
                        peak_toll: &tl,
                        tw: pad(&w, none),
                        td: pad(&d, none),
                        tw_off: pad(if off { &w_off } else { &w }, none),
                        td_off: pad(if off { &d_off } else { &d }, none),
                    };
                    let row = self.choose_row(i, &inp);
                    if assign {
                        for (&j, &dm) in dests.iter().zip(&row.car_peak) {
                            nd[self.zone_node[j as usize] as usize] += dm;
                        }
                        nd[self.zone_node[i] as usize] = 0.0; // same-node trips stay off the network
                        tree.load(g, &mut nd, &mut vol);
                    }
                    outs.push((
                        i,
                        OriginOut {
                            t,
                            k,
                            tl,
                            w,
                            d,
                            w_off,
                            d_off,
                            row,
                        },
                    ));
                    (vol, Some((tree, km, toll, srch, nd, outs)))
                },
            )
            .map(|(vol, acc)| (vol, acc.map(|a| a.5).unwrap_or_default()))
            .reduce(
                || (vec![0.0; ne], Vec::new()),
                |(mut va, mut oa), (vb, ob)| {
                    for (x, y) in va.iter_mut().zip(vb) {
                        *x += y;
                    }
                    oa.extend(ob);
                    (va, oa)
                },
            )
    }

    /// Store a whole pass's outputs: car skims, and transit skims when
    /// `transit` (else the current ones stay). Returns the choice rows.
    fn commit_outs(&mut self, mut outs: Vec<(usize, OriginOut)>, transit: bool) -> Vec<Row> {
        // Car times change: stored idle-origin totals no longer hold.
        self.idle_key = None;
        outs.sort_by_key(|(i, _)| *i);
        let mut rows = Vec::with_capacity(outs.len());
        self.car_peak.time.clear();
        self.car_peak.km.clear();
        self.car_peak.toll.clear();
        if transit {
            self.tsk_walk.clear();
            self.tsk_drive.clear();
            self.tsk_walk_off.clear();
            self.tsk_drive_off.clear();
        }
        let off_rows = outs
            .iter()
            .any(|(_, o)| !o.w_off.is_empty() || !o.d_off.is_empty())
            || (!self.net.is_empty() && self.net.offpeak_differs());
        for (_, o) in outs {
            self.car_peak.time.extend(o.t);
            self.car_peak.km.extend(o.k);
            self.car_peak.toll.extend(o.tl);
            if transit {
                self.tsk_walk.push(o.w);
                self.tsk_drive.push(o.d);
                if off_rows {
                    self.tsk_walk_off.push(o.w_off);
                    self.tsk_drive_off.push(o.d_off);
                }
            }
            rows.push(o.row);
        }
        rows
    }

    /// Car skim row for origin `i` from its shortest-path tree.
    fn car_row(
        &self,
        i: usize,
        tree: &Tree,
        km: &[f64],
        toll: &[f64],
    ) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let dests = self.pairs.dests(i);
        let n_d = dests.len();
        let mut t = vec![f32::INFINITY; n_d];
        let mut k = vec![0f32; n_d];
        let mut tl = vec![0f32; n_d];
        for (q, &j) in dests.iter().enumerate() {
            let j = j as usize;
            let n = self.zone_node[j] as usize;
            if i == j || n == self.zone_node[i] as usize {
                // Same zone or same loading node: local streets at ~25 km/h.
                let d = self.crow_km(i, j).max(self.intra_km[i]) * self.params.detour_factor;
                t[q] = (d / 25.0 * 60.0) as f32;
                k[q] = d as f32;
            } else if tree.reached(n) {
                t[q] = tree.dist[n] as f32;
                k[q] = km[n] as f32;
                tl[q] = toll[n] as f32;
            } else if self.car_free.time.len() == self.pairs.len() {
                // Past the congested tree's guard (4x the cutoff): the trip is
                // at least that slow. Keep the car option at the guard time
                // with free-flow distance and toll, rather than strand the
                // trip; it is not loaded on the roads (SIM-15).
                let f = self.pairs.row(i).start + q;
                if self.car_free.time[f].is_finite() {
                    t[q] = tree.bound as f32;
                    k[q] = self.car_free.km[f];
                    tl[q] = self.car_free.toll[f];
                }
            }
        }
        (t, k, tl)
    }

    /// Generalized cost (minutes) of each choice for origin `i`'s `q`-th kept
    /// pair, or None if unavailable.
    fn costs(&self, i: usize, q: usize, work: bool, inp: &RowInputs) -> [Option<f64>; N_CHOICES] {
        let p = &self.params;
        let k = self.pairs.row(i).start + q;
        let j = self.pairs.dest[k] as usize;
        let vot = self.vot_min[i];
        let (t, km, toll) = if work {
            (inp.peak_t[q], inp.peak_km[q], inp.peak_toll[q])
        } else {
            (
                self.car_free.time[k],
                self.car_free.km[k],
                self.car_free.toll[k],
            )
        };
        let mut c = [None; N_CHOICES];
        let t = t as f64;
        if t.is_finite() {
            let share = if work {
                p.parking_share_work
            } else {
                p.parking_share_other
            };
            let money = km as f64 * p.car_cost_per_km
                + toll as f64
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
                + o.pnr_penalty as f64
                + sd
        };
        let tw = if work { &inp.tw[q] } else { &inp.tw_off[q] };
        if tw.ok() {
            c[TRANSIT_WALK] = Some(transit(tw));
        }
        let td = if work { &inp.td[q] } else { &inp.td_off[q] };
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

    fn choose_row(&self, i: usize, inp: &RowInputs) -> Row {
        let p = &self.params;
        let row = self.pairs.row(i);
        let n_d = row.len();
        let mut r = Row {
            car_peak: vec![0.0; n_d],
            wt_work: vec![0.0; n_d],
            dt_work: vec![0.0; n_d],
            wt_other: vec![0.0; n_d],
            dt_other: vec![0.0; n_d],
            work: [0.0; N_CHOICES],
            other: [0.0; N_CHOICES],
            unserved: 0.0,
        };
        let zc = self.input.zones[i].zero_car_share;
        let cov_i = self.net.coverage[i];
        let wph = p.work_peak_hour_share();
        for (q, k) in row.enumerate() {
            let j = self.pairs.dest[k] as usize;
            for work in [true, false] {
                let trips = if work { self.hbw[k] } else { self.hbo[k] };
                if trips <= 0.0 {
                    continue;
                }
                let c = self.costs(i, q, work, inp);
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
                    r.wt_work[q] = trips * prob[TRANSIT_WALK];
                    r.dt_work[q] = trips * prob[TRANSIT_DRIVE];
                    r.car_peak[q] += trips * veh * wph;
                } else {
                    (0..N_CHOICES).for_each(|m| r.other[m] += trips * prob[m]);
                    r.wt_other[q] = trips * prob[TRANSIT_WALK];
                    r.dt_other[q] = trips * prob[TRANSIT_DRIVE];
                    r.car_peak[q] += trips * veh * p.other_peak_hour_share;
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

/// Transit skim rows (walk access, drive access) from origin `i`, on peak or
/// off-peak headways.
fn transit_row(
    s: &mut Searcher,
    i: usize,
    car_min: &[f32],
    car_km: &[f32],
    offpeak: bool,
    dests: &[u32],
) -> (Vec<TOd>, Vec<TOd>) {
    s.offpeak = offpeak;
    let read = |s: &mut Searcher, access| {
        if s.run(i, access, car_min, car_km) {
            sparse(dests.iter().map(|&j| s.od(j as usize)).collect())
        } else {
            Vec::new()
        }
    };
    let w = if s.net.walk_access[i].is_empty() {
        Vec::new()
    } else {
        read(s, Access::Walk)
    };
    let d = if s.net.pnr.is_empty() {
        Vec::new()
    } else {
        read(s, Access::Drive)
    };
    (w, d)
}

/// A sparse transit row as a full one: `none` (all `NONE`) when empty.
fn pad<'a>(v: &'a [TOd], none: &'a [TOd]) -> &'a [TOd] {
    if v.is_empty() {
        none
    } else {
        v
    }
}

/// An all-`NONE` transit row is stored empty (see [`TSkim`]).
fn sparse(v: Vec<TOd>) -> Vec<TOd> {
    if v.iter().any(|o| o.ok()) {
        v
    } else {
        Vec::new()
    }
}

/// One origin's sparse car row spread over all `z` zones (unkept pairs are
/// unreachable), for the transit search's park-and-ride drive legs.
fn dense_row(dests: &[u32], time: &[f32], km: &[f32], z: usize) -> (Vec<f32>, Vec<f32>) {
    let mut t = vec![f32::INFINITY; z];
    let mut k = vec![0f32; z];
    for ((&j, &a), &b) in dests.iter().zip(time).zip(km) {
        t[j as usize] = a;
        k[j as usize] = b;
    }
    (t, k)
}

/// Row `r` of the off-peak skim, or of the peak skim when there is no separate one.
/// Copy the service settings (not the alignment) of `from` onto `to`.
fn copy_service(from: &TransitLine, to: &mut TransitLine) {
    to.name = from.name.clone();
    to.headway_min = from.headway_min;
    to.offpeak_headway_min = from.offpeak_headway_min;
    to.fare = from.fare;
    to.fare_per_km = from.fare_per_km;
    to.speed_kph = from.speed_kph;
    to.span_hours = from.span_hours;
}

/// True if two versions of a line share mode and stop locations, so moving
/// from one to the other needs no construction.
fn same_alignment(a: &TransitLine, b: &TransitLine) -> bool {
    a.mode == b.mode
        && a.stops.len() == b.stops.len()
        && a.stops
            .iter()
            .zip(&b.stops)
            .all(|(x, y)| x.lon == y.lon && x.lat == y.lat)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After each scoped edit, redo the skims and loading in full: skims must
    /// match exactly and loads to rounding (SIM-21).
    #[test]
    fn scoped_line_edits_match_a_full_recompute() {
        use crate::synth::{generate, straight_line, SynthConfig};
        // A short drive limit, so park-and-ride users stay on their side
        // and a removal renumbers rows that are kept.
        let p = Params {
            instant_build: true,
            pnr_max_drive_min: 8.0,
            ..Params::default()
        };
        let mut s = Sim::new(generate(&SynthConfig::small()), p).unwrap();
        let check = |s: &mut Sim, what: &str| {
            let skims = format!("{:?}", [&s.tsk_walk, &s.tsk_drive]);
            let lines = s.results.lines.clone();
            s.transit_skims();
            s.finish(Instant::now(), true, None);
            assert_eq!(
                format!("{:?}", [&s.tsk_walk, &s.tsk_drive]),
                skims,
                "{what}: skims"
            );
            assert_eq!(lines.len(), s.results.lines.len(), "{what}");
            for (a, b) in lines.iter().zip(&s.results.lines) {
                let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * (1.0 + y.abs());
                assert!(
                    close(a.daily_boardings, b.daily_boardings),
                    "{what}: {} {} vs {}",
                    b.name,
                    a.daily_boardings,
                    b.daily_boardings
                );
                for (x, y) in a
                    .park_ride_cars_by_stop
                    .iter()
                    .zip(&b.park_ride_cars_by_stop)
                {
                    assert!(close(*x, *y), "{what}: {} park-and-ride", b.name);
                }
                for (x, y) in a.segment_loads.iter().zip(&b.segment_loads) {
                    assert!(
                        close(x[0], y[0]) && close(x[1], y[1]),
                        "{what}: {} segments",
                        b.name
                    );
                }
            }
        };
        // Two networks far apart: rail with park-and-ride in the west, a bus in the east.
        let rail = s
            .add_line(straight_line(
                Mode::LightRail,
                (-6.0, -6.0),
                (-6.0, 6.0),
                1.0,
                8.0,
            ))
            .unwrap();
        let east = s
            .add_line(straight_line(Mode::Bus, (6.0, -6.0), (6.0, 6.0), 0.8, 10.0))
            .unwrap();
        check(&mut s, "second network");
        s.add_line(straight_line(Mode::Bus, (3.0, 0.0), (7.0, 0.0), 0.8, 12.0))
            .unwrap();
        check(&mut s, "crossing bus joins the east network");
        let mut slower = straight_line(Mode::Bus, (6.0, -6.0), (6.0, 6.0), 0.8, 10.0);
        slower.headway_min = 20.0;
        s.update_line(east, slower).unwrap();
        check(&mut s, "east headway change");
        s.remove_line(rail).unwrap();
        check(&mut s, "first line removed (platforms renumbered)");
        s.add_line(straight_line(
            Mode::LightRail,
            (-5.0, -6.0),
            (-5.0, 6.0),
            1.0,
            8.0,
        ))
        .unwrap();
        check(
            &mut s,
            "west rail back, park-and-ride ids after the east lines",
        );
        s.preview_line(straight_line(
            Mode::Bus,
            (-7.0, 2.0),
            (-3.0, 2.0),
            0.8,
            10.0,
        ))
        .unwrap();
        check(&mut s, "after a preview");
    }

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
