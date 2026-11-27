//! sim-core behind Tauri commands.
//!
//! The Sim is built on a background thread at startup (seconds), so the window
//! opens immediately. Commands wait for it. The client keys routes by its own
//! string ids; this module maps them to sim-core's LineId.

use serde::Serialize;
use sim_core::{
    synth, Clock, Ledger, LineId, LinePreview, LineResult, Params, ScenarioInput, Sim, TransitLine,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How often the clock worker advances `Sim::tick`. Short enough that the
/// client's display (which interpolates between polls) never drifts visibly.
const TICK_EVERY: Duration = Duration::from_millis(100);

/// Parameters for the game: sim-core defaults with the construction period on
/// (lines open after their mode's build time / `construction_speedup`, F24).
fn game_params() -> Params {
    Params {
        instant_build: false,
        ..Params::default()
    }
}

/// Set TRANSPO_SCENARIO to a ScenarioInput JSON file (data-pipeline output)
/// to load it instead of the default below.
const SCENARIO_ENV: &str = "TRANSPO_SCENARIO";

/// The real Travis County build, where `npm run data` (the pipeline's
/// `scripts/download-build.sh`) puts it. Relative to this crate's source
/// directory, so it resolves in a source checkout; an installed bundle on
/// another machine falls back to the synthetic region.
const DEFAULT_SCENARIO: &str = "../../crates/data-pipeline/data/out/48453-2023/scenario.json";

#[derive(Default)]
struct Inner {
    sim: Option<Sim>,
    error: Option<String>,
    ids: HashMap<String, LineId>,
}

/// The one `Sim` of the running game, shared by all commands (Tauri managed state).
#[derive(Clone, Default)]
pub struct SimState(Arc<(Mutex<Inner>, Condvar)>);

impl SimState {
    /// Start building the Sim on a background thread and return immediately.
    pub fn start() -> Self {
        Self::start_with_params(load_scenario, game_params())
    }

    /// Instant build, so tests see lines open at once (sim-core's default
    /// became construction-on in SIM-4); tests use it.
    #[cfg(test)]
    fn start_with(load: impl FnOnce() -> Result<ScenarioInput, String> + Send + 'static) -> Self {
        let params = Params {
            instant_build: true,
            ..Params::default()
        };
        Self::start_with_params(load, params)
    }

    fn start_with_params(
        load: impl FnOnce() -> Result<ScenarioInput, String> + Send + 'static,
        params: Params,
    ) -> Self {
        let state = SimState::default();
        let s = state.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let built = load().and_then(|input| Sim::new(input, params).map_err(|e| e.to_string()));
            let (lock, cv) = &*s.0;
            let mut inner = lock.lock().unwrap();
            match built {
                Ok(sim) => {
                    eprintln!("[sim] ready in {} ms", t0.elapsed().as_millis());
                    inner.sim = Some(sim);
                }
                Err(e) => {
                    eprintln!("[sim] failed to start: {e}");
                    inner.error = Some(e);
                }
            }
            cv.notify_all();
        });
        state
    }

    /// Start the clock worker: advances `Sim::tick` by real elapsed time until
    /// the app exits. Day boundaries re-run the day and settle months inside
    /// `tick`, holding the lock, so edits wait for them (F21).
    pub fn spawn_clock(self) -> Self {
        let s = self.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            loop {
                std::thread::sleep(TICK_EVERY);
                let now = Instant::now();
                let dt = (now - last).as_secs_f64();
                last = now;
                let (lock, _) = &*s.0;
                let mut inner = lock.lock().unwrap();
                if inner.error.is_some() {
                    return;
                }
                if let Some(sim) = inner.sim.as_mut() {
                    sim.tick(dt);
                }
            }
        });
        self
    }

    /// Block until the Sim is built, then run `f` on it.
    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> Result<T, String>) -> Result<T, String> {
        let (lock, cv) = &*self.0;
        let mut inner = cv
            .wait_while(lock.lock().unwrap(), |i| {
                i.sim.is_none() && i.error.is_none()
            })
            .unwrap();
        if let Some(e) = &inner.error {
            return Err(format!("simulation failed to start: {e}"));
        }
        f(&mut inner)
    }
}

impl SimState {
    /// Add the route (new client id) or replace it (known id); results for every route.
    pub fn upsert(
        &self,
        route_id: String,
        line: TransitLine,
    ) -> Result<Vec<RouteResultOut>, String> {
        self.with(|inner| {
            let sim = inner.sim.as_mut().unwrap();
            match inner.ids.get(&route_id) {
                Some(&id) => sim.update_line(id, line).map_err(|e| e.to_string())?,
                None => {
                    let id = sim.add_line(line).map_err(|e| e.to_string())?;
                    inner.ids.insert(route_id, id);
                }
            }
            Ok(all_results(inner))
        })
    }

    /// Remove a route; unknown ids are a no-op. Results for the remaining routes.
    pub fn remove(&self, route_id: &str) -> Result<Vec<RouteResultOut>, String> {
        self.with(|inner| {
            if let Some(id) = inner.ids.remove(route_id) {
                inner
                    .sim
                    .as_mut()
                    .unwrap()
                    .remove_line(id)
                    .map_err(|e| e.to_string())?;
            }
            Ok(all_results(inner))
        })
    }
}

impl SimState {
    /// What `line` would cost and carry if built now. Nothing is charged and no
    /// line is kept. Runs under the same lock as every other call because
    /// sim-core adds and removes the line while it evaluates.
    pub fn preview(&self, line: TransitLine) -> Result<PreviewOut, String> {
        self.with(|inner| {
            let sim = inner.sim.as_mut().unwrap();
            let opens_day = sim.opening_day(line.mode);
            let preview = sim.preview_line(line).map_err(|e| e.to_string())?;
            Ok(PreviewOut { preview, opens_day })
        })
    }

    /// Lines under construction, tagged with client route ids.
    pub fn pending(&self) -> Result<Vec<PendingOut>, String> {
        self.with(|inner| {
            let by_line: HashMap<LineId, &String> =
                inner.ids.iter().map(|(k, v)| (*v, k)).collect();
            Ok(inner
                .sim
                .as_ref()
                .unwrap()
                .pending_lines()
                .into_iter()
                .filter_map(|p| {
                    Some(PendingOut {
                        route_id: (*by_line.get(&p.id)?).clone(),
                        opens_day: p.opens_day,
                        replaces_open_line: p.replaces_open_line,
                    })
                })
                .collect())
        })
    }

    /// The transit authority's books (cash, month in progress, settled months).
    pub fn ledger(&self) -> Result<LedgerOut, String> {
        self.with(|inner| Ok(ledger_out(inner.sim.as_ref().unwrap())))
    }

    /// Borrow `principal` dollars as a bond at the game's rate and term (F26).
    /// Errors once the authority is bankrupt. Returns the ledger after.
    pub fn issue_bond(&self, principal: f64) -> Result<LedgerOut, String> {
        self.with(|inner| {
            let sim = inner.sim.as_mut().unwrap();
            sim.issue_bond(principal).map_err(|e| e.to_string())?;
            Ok(ledger_out(sim))
        })
    }
}

/// sim-core's `Ledger` plus the figures the HUD needs to explain debt and
/// bankruptcy without knowing sim-core's parameters.
#[derive(Serialize, Debug)]
pub struct LedgerOut {
    /// Cash, month in progress, history, bonds, distressed months, bankrupt month.
    #[serde(flatten)]
    ledger: Ledger,
    /// Principal still owed on all bonds.
    debt: f64,
    /// Sum of the monthly payments on outstanding bonds.
    debt_service_per_month: f64,
    /// Distressed months in a row that end the game (`Params::bankruptcy_months`).
    bankruptcy_months: u32,
    /// Rate and term a new bond gets.
    bond_rate: f64,
    bond_term_months: u32,
}

fn ledger_out(sim: &Sim) -> LedgerOut {
    let l = sim.ledger();
    let p = &sim.params;
    LedgerOut {
        debt: l.debt(),
        debt_service_per_month: l.bonds.iter().map(|b| b.payment).sum(),
        bankruptcy_months: p.bankruptcy_months,
        bond_rate: p.bond_rate,
        bond_term_months: p.bond_term_months(),
        ledger: l.clone(),
    }
}

/// `Sim::preview_line` plus the day the line would open if drawn now.
#[derive(Serialize, Debug)]
pub struct PreviewOut {
    /// sim-core's preview, flattened into the same JSON object.
    #[serde(flatten)]
    preview: LinePreview,
    /// Game day (`Clock::day`) it would open: today plus its mode's build time.
    opens_day: u64,
}

/// A client route under construction (sim-core `PendingLine`).
#[derive(Serialize, Debug)]
pub struct PendingOut {
    /// The client's route id.
    route_id: String,
    /// Game day the new version enters service.
    opens_day: u64,
    /// True if an older version keeps running until then (a rebuild).
    replaces_open_line: bool,
}

/// The game clock as the client shows it.
#[derive(Serialize, Debug)]
pub struct ClockOut {
    /// sim-core's clock: `game_seconds` since day 0 00:00, `timelapse`, `paused`.
    #[serde(flatten)]
    clock: Clock,
    /// Whole game days since the start; the client re-reads results when it changes.
    day: u64,
}

impl SimState {
    /// Current clock.
    pub fn clock(&self) -> Result<ClockOut, String> {
        self.with(|inner| Ok(clock_out(inner)))
    }

    /// Pause or resume and/or set time-lapse. Either may be omitted.
    pub fn set_clock(
        &self,
        paused: Option<bool>,
        timelapse: Option<f64>,
    ) -> Result<ClockOut, String> {
        self.with(|inner| {
            let c = &mut inner.sim.as_mut().unwrap().clock;
            if let Some(p) = paused {
                c.paused = p;
            }
            if let Some(x) = timelapse {
                c.set_timelapse(x);
            }
            Ok(clock_out(inner))
        })
    }

    /// Jump to the start of next month: re-runs the day and settles the month.
    pub fn skip_to_next_month(&self) -> Result<ClockOut, String> {
        self.with(|inner| {
            inner.sim.as_mut().unwrap().skip_to_next_month();
            Ok(clock_out(inner))
        })
    }

    /// Results for every client route, without changing anything.
    pub fn results(&self) -> Result<Vec<RouteResultOut>, String> {
        self.with(|inner| Ok(all_results(inner)))
    }
}

fn clock_out(inner: &Inner) -> ClockOut {
    let clock = inner.sim.as_ref().unwrap().clock.clone();
    ClockOut {
        day: clock.day(),
        clock,
    }
}

/// One desire line: a zone pair with both centroids and its daily trips.
#[derive(Serialize, Debug)]
pub struct DesireLineOut {
    /// First zone centroid, WGS84 `[lon, lat]`.
    from: [f64; 2],
    /// Second zone centroid, WGS84 `[lon, lat]`.
    to: [f64; 2],
    /// Daily person trips between the pair, both directions, all modes.
    trips: f64,
    /// Daily transit trips between the pair, both directions.
    transit_trips: f64,
}

impl SimState {
    /// Top `n` desire lines with zone ids resolved to centroids.
    pub fn desire_lines(&self, n: usize) -> Result<Vec<DesireLineOut>, String> {
        self.with(|inner| {
            let sim = inner.sim.as_ref().unwrap();
            let at: HashMap<u32, [f64; 2]> = sim
                .input()
                .zones
                .iter()
                .map(|z| (z.id, [z.lon, z.lat]))
                .collect();
            Ok(sim
                .desire_lines(n)
                .into_iter()
                .filter_map(|d| {
                    Some(DesireLineOut {
                        from: *at.get(&d.from)?,
                        to: *at.get(&d.to)?,
                        trips: d.trips,
                        transit_trips: d.transit_trips,
                    })
                })
                .collect())
        })
    }
}

/// Scenario file to load: `TRANSPO_SCENARIO` if set (an error if it is
/// missing, since it was asked for), else the real Travis build if it has been
/// downloaded, else `None` for sim-core's synthetic Travis-sized region.
fn scenario_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(SCENARIO_ENV) {
        return Some(p.into());
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_SCENARIO);
    p.exists().then_some(p)
}

fn load_scenario() -> Result<ScenarioInput, String> {
    match scenario_path() {
        Some(path) => {
            let json =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            ScenarioInput::from_json(&json).map_err(|e| e.to_string())
        }
        None => Ok(synth::generate(&synth::SynthConfig::default())),
    }
}

/// One line's results, tagged with the client's route id.
#[derive(Serialize, Debug)]
pub struct RouteResultOut {
    /// The client's route id (`RouteSpec.id`), not sim-core's `LineId`.
    route_id: String,
    /// sim-core's results, flattened into the same JSON object.
    #[serde(flatten)]
    result: LineResult,
}

/// Startup state of the simulation, for the client to poll.
#[derive(Serialize)]
pub struct Status {
    /// The Sim is built and commands will answer without waiting.
    ready: bool,
    /// Why the build failed, if it did. Commands return this error too.
    error: Option<String>,
    /// Scenario file path (`TRANSPO_SCENARIO` or the downloaded Travis build), or `"synthetic"`.
    source: String,
}

fn all_results(inner: &Inner) -> Vec<RouteResultOut> {
    let by_line: HashMap<LineId, &String> = inner.ids.iter().map(|(k, v)| (*v, k)).collect();
    let sim = inner.sim.as_ref().unwrap();
    sim.results()
        .lines
        .iter()
        .filter_map(|r| {
            by_line.get(&r.id).map(|rid| RouteResultOut {
                route_id: (*rid).clone(),
                result: r.clone(),
            })
        })
        .collect()
}

/// Report whether the Sim is built. Never blocks.
#[tauri::command]
pub fn sim_status(state: tauri::State<'_, SimState>) -> Status {
    let inner = state.0 .0.lock().unwrap();
    Status {
        ready: inner.sim.is_some(),
        error: inner.error.clone(),
        source: scenario_path().map_or_else(|| "synthetic".into(), |p| p.display().to_string()),
    }
}

/// Add or replace a line (`routeId` from the client) and return results for every line.
/// Async so it runs off the main thread: waiting for the startup build never blocks the UI.
#[tauri::command]
pub async fn upsert_route(
    state: tauri::State<'_, SimState>,
    route_id: String,
    line: TransitLine,
) -> Result<Vec<RouteResultOut>, String> {
    let s = state.inner().clone();
    blocking(move || s.upsert(route_id, line)).await
}

/// Remove a line and return results for the remaining lines.
#[tauri::command]
pub async fn remove_route(
    state: tauri::State<'_, SimState>,
    route_id: String,
) -> Result<Vec<RouteResultOut>, String> {
    let s = state.inner().clone();
    blocking(move || s.remove(&route_id)).await
}

/// Top `n` zone pairs by daily trips, with endpoint coordinates (F06 desire lines).
#[tauri::command]
pub async fn desire_lines(
    state: tauri::State<'_, SimState>,
    n: usize,
) -> Result<Vec<DesireLineOut>, String> {
    let s = state.inner().clone();
    blocking(move || s.desire_lines(n)).await
}

/// Cost, ridership and opening day of a line before it is built (F16 build cost preview).
#[tauri::command]
pub async fn preview_route(
    state: tauri::State<'_, SimState>,
    line: TransitLine,
) -> Result<PreviewOut, String> {
    let s = state.inner().clone();
    blocking(move || s.preview(line)).await
}

/// Routes under construction and the day each opens (F24).
#[tauri::command]
pub async fn pending_routes(state: tauri::State<'_, SimState>) -> Result<Vec<PendingOut>, String> {
    let s = state.inner().clone();
    blocking(move || s.pending()).await
}

/// The ledger: cash and this month's capital spend and grants.
#[tauri::command]
pub async fn ledger(state: tauri::State<'_, SimState>) -> Result<LedgerOut, String> {
    let s = state.inner().clone();
    blocking(move || s.ledger()).await
}

/// Issue a bond for `principal` dollars; returns the ledger after (F26).
#[tauri::command]
pub async fn issue_bond(
    state: tauri::State<'_, SimState>,
    principal: f64,
) -> Result<LedgerOut, String> {
    let s = state.inner().clone();
    blocking(move || s.issue_bond(principal)).await
}

/// Current game clock (F21). The client polls it and interpolates in between.
#[tauri::command]
pub async fn clock(state: tauri::State<'_, SimState>) -> Result<ClockOut, String> {
    let s = state.inner().clone();
    blocking(move || s.clock()).await
}

/// Pause/resume and set time-lapse; omitted fields are unchanged.
#[tauri::command]
pub async fn set_clock(
    state: tauri::State<'_, SimState>,
    paused: Option<bool>,
    timelapse: Option<f64>,
) -> Result<ClockOut, String> {
    let s = state.inner().clone();
    blocking(move || s.set_clock(paused, timelapse)).await
}

/// Skip to the first day of next month, settling the month's budget.
#[tauri::command]
pub async fn skip_to_next_month(state: tauri::State<'_, SimState>) -> Result<ClockOut, String> {
    let s = state.inner().clone();
    blocking(move || s.skip_to_next_month()).await
}

/// Results for every route, for re-reading after the day re-runs.
#[tauri::command]
pub async fn results(state: tauri::State<'_, SimState>) -> Result<Vec<RouteResultOut>, String> {
    let s = state.inner().clone();
    blocking(move || s.results()).await
}

/// Run a closure that may wait on the Condvar or the Sim lock on Tauri's blocking
/// pool, so async runtime workers are never parked (review follow-up on #9).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("sim task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(json: &str) -> TransitLine {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn upsert_update_remove_round_trip() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())));
        let a = line(
            r#"{"name":"A","mode":"bus","headway_min":15,
            "stops":[{"lon":-97.76,"lat":30.25},{"lon":-97.743,"lat":30.268},{"lon":-97.73,"lat":30.29}]}"#,
        );
        let out = state.upsert("r1".into(), a.clone()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].route_id, "r1");
        assert_eq!(out[0].result.boardings_by_stop.len(), 3);

        // Same client id updates in place instead of adding a second line.
        let mut faster = a;
        faster.headway_min = 5.0;
        let out = state.upsert("r1".into(), faster).unwrap();
        assert_eq!(out.len(), 1);

        let out = state.remove("r1").unwrap();
        assert!(out.is_empty());
        assert!(state.remove("missing").unwrap().is_empty());
    }

    #[test]
    fn preview_charges_nothing_and_build_does() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())));
        let rail = line(
            r#"{"name":"R","mode":"light_rail","headway_min":10,
            "stops":[{"lon":-97.76,"lat":30.25},{"lon":-97.743,"lat":30.268},{"lon":-97.73,"lat":30.29}]}"#,
        );
        let cash0 = state.ledger().unwrap().ledger.cash;
        let p = state.preview(rail.clone()).unwrap().preview;
        assert!(p.capital_cost > 0.0 && p.line.daily_boardings > 0.0);
        assert_eq!(
            state.ledger().unwrap().ledger.cash,
            cash0,
            "preview is free"
        );
        assert!(
            state.remove("none").unwrap().is_empty(),
            "preview kept no line"
        );

        state.upsert("r1".into(), rail).unwrap();
        let l = state.ledger().unwrap().ledger;
        assert!((l.current.capital_spent - p.capital_cost).abs() < 1.0);
        assert!((cash0 - l.cash - (p.capital_cost - p.federal_grant)).abs() < 1.0);
    }

    #[test]
    fn clock_controls_and_month_skip_settle_the_budget() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())));
        let c = state.set_clock(Some(true), Some(10.0)).unwrap();
        assert!(c.clock.paused);
        assert_eq!(c.clock.timelapse, 10.0);
        let c = state.set_clock(None, Some(-1.0)).unwrap();
        assert_eq!(c.clock.timelapse, 10.0, "invalid time-lapse ignored");

        let bus = line(
            r#"{"name":"B","mode":"bus","headway_min":10,
            "stops":[{"lon":-97.76,"lat":30.25},{"lon":-97.743,"lat":30.268},{"lon":-97.73,"lat":30.29}]}"#,
        );
        state.upsert("b".into(), bus).unwrap();
        let c = state.skip_to_next_month().unwrap();
        assert_eq!(c.day, 30);
        assert_eq!(
            state.ledger().unwrap().ledger.history.len(),
            1,
            "month settled"
        );
        assert_eq!(state.results().unwrap().len(), 1);
    }

    #[test]
    fn clock_worker_advances_time() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())))
            .spawn_clock();
        let t0 = state
            .set_clock(Some(false), Some(60.0))
            .unwrap()
            .clock
            .game_seconds;
        std::thread::sleep(Duration::from_millis(350));
        let t1 = state.clock().unwrap().clock.game_seconds;
        // 60x: one real second is one game hour; allow for scheduling slack.
        assert!(t1 - t0 > 600.0, "advanced {} game s", t1 - t0);
    }

    #[test]
    fn construction_period_holds_new_lines_until_they_open() {
        let state = SimState::start_with_params(
            || Ok(synth::generate(&synth::SynthConfig::small())),
            game_params(),
        );
        let rail = line(
            r#"{"name":"R","mode":"light_rail","headway_min":10,
            "stops":[{"lon":-97.76,"lat":30.25},{"lon":-97.743,"lat":30.268},{"lon":-97.73,"lat":30.29}]}"#,
        );
        let p = state.preview(rail.clone()).unwrap();
        assert!(
            p.opens_day > 30,
            "light rail takes months: day {}",
            p.opens_day
        );

        assert!(
            state.upsert("r1".into(), rail).unwrap().is_empty(),
            "not open yet"
        );
        let pending = state.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].route_id, "r1");
        assert_eq!(pending[0].opens_day, p.opens_day);
        assert!(!pending[0].replaces_open_line);

        while state.clock().unwrap().day < p.opens_day {
            state.skip_to_next_month().unwrap();
        }
        assert!(state.pending().unwrap().is_empty());
        assert_eq!(state.results().unwrap().len(), 1, "open now");
    }

    #[test]
    fn bonds_add_cash_and_debt_and_stop_at_bankruptcy() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())));
        // No lines: no tax area, nothing pledged, so nothing can be borrowed (SIM-9).
        assert!(state.issue_bond(1e6).unwrap_err().contains("capacity"));
        // A light rail line across the region leaves cash far below zero.
        let rail = line(
            r#"{"name":"LR","mode":"light_rail","headway_min":10,
            "stops":[{"lon":-97.80,"lat":30.27},{"lon":-97.76,"lat":30.27},{"lon":-97.72,"lat":30.27},{"lon":-97.68,"lat":30.27}]}"#,
        );
        state.upsert("r1".into(), rail).unwrap();
        let cash0 = state.ledger().unwrap().ledger.cash;
        assert!(cash0 < 0.0);
        let l = state.issue_bond(1e6).unwrap();
        assert!((l.ledger.cash - cash0 - 1e6).abs() < 1.0);
        assert!((l.debt - 1e6).abs() < 1.0);
        assert!(l.debt_service_per_month > 0.0 && l.bond_term_months > 0);
        assert!(state.issue_bond(-5.0).is_err());
        assert!(state.issue_bond(50e9).unwrap_err().contains("capacity"));

        // Cash stays below zero every month, so bankruptcy follows.
        let months = state.ledger().unwrap().bankruptcy_months;
        for _ in 0..=months {
            state.skip_to_next_month().unwrap();
        }
        let l = state.ledger().unwrap();
        assert!(
            l.ledger.bankrupt_month.is_some(),
            "{} distressed",
            l.ledger.distressed_months
        );
        assert!(state.issue_bond(1e6).unwrap_err().contains("bankrupt"));
    }

    #[test]
    fn desire_lines_resolve_to_zone_centroids() {
        let state = SimState::start_with(|| Ok(synth::generate(&synth::SynthConfig::small())));
        let lines = state.desire_lines(10).unwrap();
        assert!(!lines.is_empty() && lines.len() <= 10);
        assert!(
            lines.windows(2).all(|w| w[0].trips >= w[1].trips),
            "sorted by trips"
        );
        // Synthetic region is centered on downtown Austin.
        assert!(lines
            .iter()
            .all(|d| (d.from[0] + 97.7).abs() < 1.0 && (d.from[1] - 30.3).abs() < 1.0));
    }

    #[test]
    fn load_failure_is_reported_not_hung() {
        let state = SimState::start_with(|| Err("no file".into()));
        let err = state.remove("r1").unwrap_err();
        assert!(err.contains("no file"));
    }
}
