# sim-core

Aggregate zone-to-zone travel model for Transpo-Sim. It turns a region's zones,
road network and commute data into daily trips, chooses a mode for each trip,
loads cars onto roads and riders onto transit lines, and re-evaluates ridership
in milliseconds when the player draws or edits a line. Headless Rust library:
the Tauri app calls it in-process, the data pipeline produces its input.

| Doc | What it covers |
|---|---|
| this README | input schema, public API, how to run, performance |
| [docs/MODEL.md](docs/MODEL.md) | the model math step by step, every parameter, and why each simplification was made |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | modules, data flow, what is recomputed when, the clock, threading, how to extend |

Design source: the approved Game Overview doc (aggregate simulation is the truth,
agents are the close-up; Travis County is the M0/M1 tuning bench; line edit
re-evaluates in under 2 s; one clock, time-lapse is a multiplier; no hyperloop).

## Status (M0/M1)

Built: input schema and validation, synthetic Travis-like scenario generator,
road graph and congested car assignment, home-to-work and non-work trip
generation, desired work arrival times, six-way logit mode choice with real-world
costs and fares, transit path search with walk and park-and-ride access and
transfers, per-line ridership and economics, the shared game clock.

Not built yet: calibration against real Travis data (needs the pipeline), crowding
in transit choice, local-tier agents (M2), budget ledger (M2), statewide scale
(M3). See [docs/MODEL.md#known-gaps](docs/MODEL.md#known-gaps).

## Run it

```sh
cargo test -p sim-core                                  # unit + integration tests
cargo run --release -p sim-core --example bench         # Travis-sized synthetic benchmark
cargo run --release -p sim-core --example bench 0.25    # denser road grid (40k nodes)
```

## Performance (4-thread container, release build, synthetic Travis-sized region)

676 zones, 1.3M residents, 4.1M daily trips.

| Operation | 10k road nodes | 40k road nodes | Target |
|---|---|---|---|
| `Sim::new` (baseline to equilibrium) | 1.5 s | 6.4 s | none (load time) |
| `add_line` / `update_line` / `remove_line` | 0.02 s | 0.02 s | < 2 s |
| `equilibrate` (runs once per game day) | 1.2 s | 5.5 s | < 24 s (a day at 60x) |

Line edits are fast because they leave car travel times alone and only redo
transit paths, mode choice and transit loading. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#what-is-recomputed-when).

## Public API

Everything the app needs is on `Sim`. All result types are `serde::Serialize`,
so a Tauri command can return them directly as JSON.

```rust
use sim_core::{Params, ScenarioInput, Sim, TransitLine, Mode};

let mut sim = Sim::from_json(&std::fs::read_to_string("travis.json")?, Params::default())?;

// Draw a line: returns a stable id. Results for ALL lines are refreshed.
let id = sim.add_line(line)?;
for l in &sim.results().lines {
    println!("{} riders/day, per stop {:?}", l.daily_boardings, l.boardings_by_stop);
}
sim.update_line(id, edited_line)?;   // change stops, headway, fare...
sim.remove_line(id)?;

// Game loop: advance the one clock; a game-day boundary re-runs the full equilibrium.
sim.clock.set_timelapse(10.0);
let days_crossed = sim.tick(real_dt_seconds);
```

| Call | Cost | What it does |
|---|---|---|
| `Sim::new(input, params)` / `Sim::from_json(json, params)` | seconds | Validates input, builds networks, runs the baseline to equilibrium. Lines in the input become lines 0..n. |
| `add_line(TransitLine) -> LineId` | ~20 ms | Adds a line, refreshes `results()`. Ids are never reused. |
| `update_line(LineId, TransitLine)` | ~20 ms | Replaces a line in place (same id). |
| `remove_line(LineId)` | ~20 ms | Removes a line. Its id is retired. |
| `lines() -> Vec<(LineId, &TransitLine)>` | free | Active lines. |
| `results() -> &Results` | free | Latest results (see below). |
| `zone_results() -> Vec<ZoneResult>` | ms | Per zone: daily trips, transit trips and share, stop coverage. For heatmaps. |
| `link_flows() -> Vec<LinkFlow>` | ms | Per directed road edge: peak-hour volume, v/c, congested speed. For congestion bands. |
| `desire_lines(n) -> Vec<DesireLine>` | ms | Top n zone pairs by daily trips, with transit trips. For the M0 desire-line view. |
| `equilibrate()` | seconds | Full mode choice and congested car assignment. `tick` calls it at day boundaries. |
| `tick(real_dt) -> u64` | free or seconds | Advances `sim.clock`; re-equilibrates once per game day crossed. Call from a worker thread. |
| `calibrate_mode_shares(ModeSplit, iters)` | ~0.1 s/iter | Fits the logit constants to target mode shares. |
| `input() -> &ScenarioInput` | free | The scenario as loaded (lines live in `lines()`). |
| `sim.params`, `sim.clock` | | Public fields. Change params then call `equilibrate()`. |

`Results` (refreshed by every call that changes state):

| Field | Meaning |
|---|---|
| `daily_trips`, `work_trips`, `other_trips` | Person trips per day by choice (`ModeSplit`: drive, carpool, transit_walk, transit_drive, walk, bike) |
| `mode_share` | `daily_trips` as shares |
| `unserved_trips` | Trips with no available option (no car, no transit, too far to walk) |
| `lines: Vec<LineResult>` | One per active line, in id order |
| `peak_vehicle_km`, `peak_avg_speed_kph` | Road network health in the AM peak hour |
| `work_peak_hour_share` | Share of commutes arriving in the busiest hour |
| `car_times_stale` | True after a line edit until the next `equilibrate` (car congestion not yet updated) |
| `eval_ms` | Wall time of the call that produced these results |

`LineResult`: `id, name, mode, length_km, run_min, daily_boardings,
boardings_by_stop` (aligned with the line's `stops`), `passenger_km,
max_segment_load, peak_hour_load, peak_hour_capacity, vehicles_required,
revenue_hours_per_day, operating_cost_per_day, fare_revenue_per_day,
farebox_recovery, segment_loads` (`[forward, reverse]` per stop pair).

Errors are `SimError(String)` with a readable message (bad JSON, dangling ids,
a line with one stop, an unknown line id).

**Threading.** `Sim` is `Send`; wrap it in a `Mutex` behind Tauri commands. Line
edits are short enough to run on the command thread. `tick` and `equilibrate`
take seconds, so run them on a worker.

**Synthetic data.** `sim_core::synth::generate(&SynthConfig::default())` builds a
Travis-sized region (676 zones, 1.3M people, freeway grid) and
`synth::straight_line(...)` builds a test line. The app can develop against this
until the pipeline lands.

## Input schema (contract with `data-pipeline`)

One JSON file per region, deserialized into `ScenarioInput`
([src/schema.rs](src/schema.rs) is the source of truth). Coordinates are WGS84
degrees, money is 2024 USD, ids are `u32` (unique per list, need not be dense).
Fields marked optional may be omitted.

```jsonc
{
  "zones": [{
    "id": 1, "lon": -97.75, "lat": 30.27,   // centroid
    "population": 2000,                      // residents
    "jobs": 6000,                            // LODES WAC jobs in zone
    "road_node": 10,                         // nearest drivable road node id
    "zero_car_share": 0.06,                  // optional, ACS B08201
    "median_income": 70000,                  // optional, ACS B19013
    "parking_cost": 0,                       // optional, $/day at destination
    "area_km2": 2.1                          // optional, else estimated from spacing
  }],
  "road_nodes": [{ "id": 10, "lon": -97.75, "lat": 30.27 }],
  "road_links": [{
    "from": 10, "to": 20, "length_m": 6200, "speed_kph": 60,
    "lanes": 1,            // optional, per direction
    "capacity_vph": 1800,  // optional, per direction; derived from speed x lanes
    "toll": 0,             // optional, $ per traversal
    "oneway": false        // optional; false = usable both ways
  }],
  "commutes": [{ "home": 2, "work": 1, "workers": 900 }],  // LODES OD by zone
  "lines": []              // optional, transit present at start (see TransitLine)
}
```

`TransitLine`: `{ "name"?, "mode", "stops": [{ "lon", "lat", "name"?, "park_ride"? }],
"headway_min", "fare"?, "fare_per_km"?, "speed_kph"?, "span_hours"? }`. Omitted
values take per-mode defaults ([src/params.rs](src/params.rs) `mode_defaults`).
Modes: `bus, trolleybus, brt, tram, light_rail, commuter_rail, metro,
high_speed_rail, monorail, people_mover, gondola, maglev, ferry`. Hyperloop is
deliberately absent.

`ScenarioInput::validate()` checks ids, ranges and NaN, self-loop links, and
zero-lane links without an explicit capacity; the pipeline should run it on its
output. A small working example is [tests/fixtures/tiny.json](tests/fixtures/tiny.json).

M0/M1 zones are Travis County block groups. Keep the road graph simplified
(degree-2 nodes contracted); tens of thousands of nodes is fine (see the
performance table).
