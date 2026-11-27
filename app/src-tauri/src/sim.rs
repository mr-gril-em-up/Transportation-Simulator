//! sim-core behind Tauri commands.
//!
//! The Sim is built on a background thread at startup (seconds), so the window
//! opens immediately. Commands wait for it. The client keys routes by its own
//! string ids; this module maps them to sim-core's LineId.

use serde::Serialize;
use sim_core::{
    synth, Ledger, LineId, LinePreview, LineResult, Params, ScenarioInput, Sim, TransitLine,
};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

/// Set TRANSPO_SCENARIO to a ScenarioInput JSON file (data-pipeline output).
/// Without it the app uses sim-core's synthetic Travis-sized region.
const SCENARIO_ENV: &str = "TRANSPO_SCENARIO";

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
        Self::start_with(load_scenario)
    }

    fn start_with(load: impl FnOnce() -> Result<ScenarioInput, String> + Send + 'static) -> Self {
        let state = SimState::default();
        let s = state.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let built = load()
                .and_then(|input| Sim::new(input, Params::default()).map_err(|e| e.to_string()));
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
    pub fn preview(&self, line: TransitLine) -> Result<LinePreview, String> {
        self.with(|inner| {
            inner
                .sim
                .as_mut()
                .unwrap()
                .preview_line(line)
                .map_err(|e| e.to_string())
        })
    }

    /// The transit authority's books (cash, month in progress, settled months).
    pub fn ledger(&self) -> Result<Ledger, String> {
        self.with(|inner| Ok(inner.sim.as_ref().unwrap().ledger().clone()))
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

fn load_scenario() -> Result<ScenarioInput, String> {
    match std::env::var(SCENARIO_ENV) {
        Ok(path) => {
            let json = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
            ScenarioInput::from_json(&json).map_err(|e| e.to_string())
        }
        Err(_) => Ok(synth::generate(&synth::SynthConfig::default())),
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
    /// `TRANSPO_SCENARIO` path, or `"synthetic"`.
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
        source: std::env::var(SCENARIO_ENV).unwrap_or_else(|_| "synthetic".into()),
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

/// Cost and ridership of a line before it is built (F16 build cost preview).
#[tauri::command]
pub async fn preview_route(
    state: tauri::State<'_, SimState>,
    line: TransitLine,
) -> Result<LinePreview, String> {
    let s = state.inner().clone();
    blocking(move || s.preview(line)).await
}

/// The ledger: cash and this month's capital spend and grants.
#[tauri::command]
pub async fn ledger(state: tauri::State<'_, SimState>) -> Result<Ledger, String> {
    let s = state.inner().clone();
    blocking(move || s.ledger()).await
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
        let cash0 = state.ledger().unwrap().cash;
        let p = state.preview(rail.clone()).unwrap();
        assert!(p.capital_cost > 0.0 && p.line.daily_boardings > 0.0);
        assert_eq!(state.ledger().unwrap().cash, cash0, "preview is free");
        assert!(
            state.remove("none").unwrap().is_empty(),
            "preview kept no line"
        );

        state.upsert("r1".into(), rail).unwrap();
        let l = state.ledger().unwrap();
        assert!((l.current.capital_spent - p.capital_cost).abs() < 1.0);
        assert!((cash0 - l.cash - (p.capital_cost - p.federal_grant)).abs() < 1.0);
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
