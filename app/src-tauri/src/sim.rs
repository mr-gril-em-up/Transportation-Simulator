//! sim-core behind Tauri commands.
//!
//! The Sim is built on a background thread at startup (seconds), so the window
//! opens immediately. Commands wait for it. The client keys routes by its own
//! string ids; this module maps them to sim-core's LineId.

use serde::Serialize;
use sim_core::{synth, LineId, LineResult, Params, ScenarioInput, Sim, TransitLine};
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
            let built = load().and_then(|input| {
                Sim::new(input, Params::default()).map_err(|e| e.to_string())
            });
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
            .wait_while(lock.lock().unwrap(), |i| i.sim.is_none() && i.error.is_none())
            .unwrap();
        if let Some(e) = &inner.error {
            return Err(format!("simulation failed to start: {e}"));
        }
        f(&mut inner)
    }
}

impl SimState {
    /// Add the route (new client id) or replace it (known id); results for every route.
    pub fn upsert(&self, route_id: String, line: TransitLine) -> Result<Vec<RouteResultOut>, String> {
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
                inner.sim.as_mut().unwrap().remove_line(id).map_err(|e| e.to_string())?;
            }
            Ok(all_results(inner))
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
            by_line.get(&r.id).map(|rid| RouteResultOut { route_id: (*rid).clone(), result: r.clone() })
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
    state.upsert(route_id, line)
}

/// Remove a line and return results for the remaining lines.
#[tauri::command]
pub async fn remove_route(
    state: tauri::State<'_, SimState>,
    route_id: String,
) -> Result<Vec<RouteResultOut>, String> {
    state.remove(&route_id)
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
        let a = line(r#"{"name":"A","mode":"bus","headway_min":15,
            "stops":[{"lon":-97.76,"lat":30.25},{"lon":-97.743,"lat":30.268},{"lon":-97.73,"lat":30.29}]}"#);
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
    fn load_failure_is_reported_not_hung() {
        let state = SimState::start_with(|| Err("no file".into()));
        let err = state.remove("r1").unwrap_err();
        assert!(err.contains("no file"));
    }
}
