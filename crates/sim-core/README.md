# sim-core

Aggregate zone-to-zone travel model for Transpo-Sim. It turns a region's zones,
road network and commute data into daily trips, chooses a mode for each trip,
loads cars onto roads and riders onto transit lines, and re-evaluates ridership
in milliseconds when the player draws or edits a line. Headless Rust library:
the Tauri app calls it in-process, the data pipeline produces its input.
Delivers roadmap features F01, F11, F12, F18 and the sim-core side of F15, and
parts of F13, F14, F21, F22, F23, F24, F25 and F26 (see [Status](#status)).

| Doc | What it covers |
|---|---|
| this README | input schema, public API, how to run, performance |
| [docs/MODEL.md](docs/MODEL.md) | the model math step by step, every parameter, and why each simplification was made |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | modules, data flow, what is recomputed when, the clock, threading, how to extend |

Design source: the approved Game Overview doc (aggregate simulation is the truth,
agents are the close-up; Travis County is the M0/M1 tuning bench; line edit
re-evaluates in under 2 s; one clock, time-lapse is a multiplier; no hyperloop).
## Run

```sh
cargo run --release -p sim-core --example bench         # Travis-sized synthetic benchmark
cargo run --release -p sim-core --example bench 0.25    # denser road grid (40k nodes)
```

The library has no binary of its own; the app links it (`Sim` below). The bench
builds a synthetic region (`synth::generate`), runs the baseline, calibrates mode
shares, adds two lines and prints timings. It panics if a line edit takes 2 s or
more.

## Test

```sh
cargo test -p sim-core
cargo doc -p sim-core --no-deps      # public items are documented (missing_docs is on)
```

29 tests, under a second in debug builds. `tests/schema.rs` loads the fixture
[tests/fixtures/tiny.json](tests/fixtures/tiny.json) and checks defaults and
validation errors. `tests/model.rs` runs the full model on
`SynthConfig::small()` (64 zones) and checks trip conservation, that a line adds
riders and shifts mode share, that higher fares and longer headways lose riders,
stable line ids, transfers, congestion feedback, calibration, the clock,
that results serialize, and the money: preview matches build and charges
nothing, extensions pay only the difference, months settle fares, costs and
local tax. Unit tests cover Dijkstra and loading, stop coverage,
the logit, distances and the clock. `tests/figures.rs` checks that sourced
defaults equal their rows in `crates/data-pipeline/reference/figures.csv`. Full list in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#testing).

## Interfaces

### Public API (`Sim`)

Everything the app needs is on `Sim`. All result types are `serde::Serialize`,
so a Tauri command can return them directly as JSON.

```rust
use sim_core::{Params, ScenarioInput, Sim, TransitLine, Mode};

let mut sim = Sim::from_json(&std::fs::read_to_string("travis.json")?, Params::default())?;

// Draw a line: returns a stable id. Capital is charged now. With
// params.instant_build = false (the game's setting; the default stays true until
// the app shows pending lines, SIM-1) the line is under construction until
// sim.opening_day(mode) (light rail: 6 real years / 12 = ~183 game days),
// listed in results().pending_lines, then carries riders.
let id = sim.add_line(line)?;
for l in &sim.results().lines {
    println!("{} riders/day, per stop {:?}", l.daily_boardings, l.boardings_by_stop);
}
sim.update_line(id, edited_line)?;   // change stops, headway, fare...
sim.remove_line(id)?;

// Build preview: cost and riders before committing. Charges nothing.
let p = sim.preview_line(candidate.clone())?;
println!("${:.0}M, grant ${:.0}M, {} riders/day", p.capital_cost / 1e6, p.federal_grant / 1e6, p.line.daily_boardings);
println!("cash ${:.0}M", sim.ledger().cash / 1e6);  // add/update_line charge capital

// Game loop: advance the one clock; a game-day boundary re-runs the full equilibrium.
sim.clock.set_timelapse(10.0);
let days_crossed = sim.tick(real_dt_seconds);
```

| Call | Cost | What it does |
|---|---|---|
| `Sim::new(input, params)` / `Sim::from_json(json, params)` | seconds | Validates input, builds networks, runs the baseline to equilibrium. Lines in the input become lines 0..n. |
| `add_line(TransitLine) -> LineId` | 40-80 ms (20-40 ms with `instant_build`) | Adds a line and charges its capital (less any federal grant on its forecast). It enters service on `opening_day(mode)`; until then it is in `pending_lines()` with no riders. With `params.instant_build` (default true for now, SIM-1) it opens at once and `results()` is refreshed. Ids are never reused. |
| `opening_day(Mode) -> u64` | free | Game day a line drawn today would open: `build_years` for the mode / `construction_speedup`. |
| `pending_lines() -> Vec<PendingLine>` | free | Lines (or new versions) under construction: `id, name, mode, opens_day, replaces_open_line`. Also in `results().pending_lines`. |
| `update_line(LineId, TransitLine)` | 20-80 ms | Replaces a line (same id). Service changes (headway, fare, speed, span, name) apply at once. Moving, adding or removing stops, or changing mode, is construction: the extra capital is charged now, the old version keeps running, the new one opens on `opening_day`. |
| `remove_line(LineId)` | 20-40 ms | Removes a line, open or under construction. Its id is retired. No refund. |
| `preview_line(TransitLine) -> LinePreview` | 40-80 ms | Build preview: ridership (`line: LineResult`), `capital_cost`, `federal_grant`, `capital_per_daily_boarding`. Evaluates the line, then removes it; ledger and ids unchanged. |
| `ledger() -> &Ledger` | free | Transit authority books: `cash`, `current` month, settled `history` (`MonthStatement`), `served_population`. `add_line`/`update_line` charge capital (net of any federal grant); month boundaries settle fares, operating cost, assistance and local tax. |
| `skip_to_next_month() -> u64` | ~2 s | Jumps the clock to next month, re-runs the day, settles the month. Use this, not `sim.clock.skip_to_next_month()`, so the ledger settles. |
| `lines() -> Vec<(LineId, &TransitLine)>` | free | Every line as last drawn (the version under construction if any). `results().lines` has only lines in service. |
| `results() -> &Results` | free | Latest results (see below). |
| `zone_results() -> Vec<ZoneResult>` | ms | Per zone: daily trips, transit trips and share, stop coverage. For heatmaps. |
| `link_flows() -> Vec<LinkFlow>` | ms | Per directed road edge: peak-hour volume, v/c, congested speed. For congestion bands. |
| `desire_lines(n) -> Vec<DesireLine>` | ms | Top n zone pairs by daily trips, with transit trips. For the M0 desire-line view. |
| `equilibrate()` | seconds | Full mode choice and congested car assignment from free-flow times. Call after changing `params`. |
| `equilibrate_warm()` | ~2 s at Travis size | Daily re-run starting from the previous day's road volumes (`daily_iterations` rounds). |
| `tick(real_dt) -> u64` | free or ~2 s | Advances `sim.clock`; at a day boundary opens lines whose construction finished, runs `equilibrate_warm` once, and settles the ledger at month boundaries. Call from a worker thread. |
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
| `car_times_stale` | True after a line edit until the next `equilibrate` or `equilibrate_warm` (car congestion not yet updated) |
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

### Input schema (contract with `data-pipeline`)

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

### Performance (4-thread container, release build, synthetic Travis-sized region)

676 zones, 1.3M residents, 4.1M daily trips.

| Operation | 10k road nodes | 40k road nodes | 92k road nodes | Target |
|---|---|---|---|---|
| `Sim::new` (baseline to equilibrium) | 0.9 s | 3.0 s | 6.8 s | none (load time) |
| `add_line` / `update_line` / `remove_line` | 0.04 s | 0.04 s | 0.04 s | < 2 s |
| `equilibrate_warm` (what `tick` runs once per game day) | 0.3 s | 0.9 s | 2.1 s | < 24 s (a day at 60x) |
| `equilibrate` (full re-run from free flow) | 0.7 s | 2.3 s | 5.2 s | none (after a params change) |

The 92k column (`bench 0.165`) matches the size of the real Travis build
(766 zones, 93k road nodes). Road-side cost grows linearly with road nodes;
line edits do not touch the road graph, so they stay flat. Before this PR
the same region took 15.2 s to load and 13.0 s per game day; the gains come
from one shortest-path tree per origin per round instead of two, warm-starting
the daily run, and a leaner Dijkstra (see
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#threading-and-performance)).
Contracting degree-2 road nodes is the next lever for the real OSM graph.

Line edits are fast because they leave car travel times alone and only redo
transit paths, mode choice and transit loading. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#what-is-recomputed-when).
## Design and rationale

The full reasoning is in [docs/MODEL.md](docs/MODEL.md) (math, parameter sources,
why each simplification) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The
choices a reader is most likely to question:

- **Zones, not people.** The design doc's rule is that aggregate flows are the
  truth and agents are only the close-up (D7). Zone-to-zone matrices are what
  the score uses and are fast enough to re-run on every edit. Agents come in M2
  (F28), sampled from these flows.
- **Line edits do not re-run road congestion.** A new line moves car volumes a
  few percent; the player needs instant feedback. Car times catch up at the next
  game-day boundary, the strategic tier's cadence. `results.car_times_stale`
  flags the gap. This is how a line edit takes 20 to 40 ms instead of seconds.
- **Method of successive averages with mode-choice feedback** instead of
  Frank-Wolfe: same equilibrium, no line search, and mode choice sits in the
  same loop.
- **Multinomial logit, not nested.** Nesting parameters cannot be calibrated
  yet; the MNL error (overstating pull from walk and bike) is small in Texas.
- **Stop coverage as a share of each zone**, not centroid distance, so ridership
  does not jump when a stop moves 100 m.
- **Constants live in `Params` with their source in a comment**, not read from a
  data file at run time, so sim-core stays headless and self-contained (repo
  rule). Where the pipeline's reference table
  ([figures.csv](../data-pipeline/reference/figures.csv)) has a figure, the
  default copies it and [tests/figures.rs](tests/figures.rs) fails if the two
  drift: trip rate 2.28/person/day, bus/light rail/commuter rail cost per car
  hour, CapMetro fares, and the 2x walk and wait weights (D25, D29).
- **Dense Z x Z matrices.** Right for ~700 block groups; statewide (F34) needs
  sparse storage, noted in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#threading-and-performance).

## Known gaps

| Gap | Effect | Fixed by |
|---|---|---|
| Not calibrated to real Travis data | Mode shares and ridership are plausible, not validated | F14 (ACS shares), F19 (Red Line within 30%) |
| Non-work trips use one purpose and a jobs + residents attraction, distributed on car time | No purpose split; transit lines do not pull destinations | F13; logsum destination choice |
| No departure-time choice; one AM peak hour assigned | No peak spreading; midday congestion not modeled | F22, regional tier F35 |
| No crowding in transit choice | Overloaded lines keep their riders (load is reported vs capacity) | F14 follow-up |
| All-or-nothing transit assignment | Parallel lines on one corridor do not share riders by frequency | F14 follow-up |
| Fare revenue counts each boarding at full fare | Overstates revenue when transfers are free | F25 |
| Park-and-ride car legs are not loaded on roads | Slight undercount of road volume near P&R lots | F14 follow-up |
| Capital cost has no terrain or right-of-way multipliers; all capital is charged when construction starts | Same cost everywhere; no spending spread over the build | F23, F24 follow-up |
| No road capacity loss during construction; build time does not grow with length | A 5 km and a 30 km line take the same time | F24 follow-up |
| No bonds, state appropriation or bankruptcy | Cash can go negative without consequence | F26 |
| Dense matrices | Memory grows with zones squared | F34 |

## Status

| Feature | Status in sim-core |
|---|---|
| F01 Workspace and input schema | Done (#1; NaN, zero-lane and self-loop validation added here) |
| F11 Car travel times and BPR assignment | Done |
| F12 Transit skims (walk, wait, in-vehicle, transfers, park-and-ride) | Done |
| F13 Non-work trips | Partial: gravity model, single purpose; total rate 2.28/person/day (2022 NHTS) minus work trips |
| F14 Logit mode split | Partial: six-way MNL and `calibrate_mode_shares` done; ACS calibration needs pipeline data |
| F15 Sim API | sim-core side done; Tauri commands are the app's |
| F18 Incremental re-evaluation | Done on the synthetic Travis-sized region (20-40 ms); confirm on real Travis data |
| F21 One clock | Partial: `Clock` with time-lapse and skip-to-next-month in sim-core; app still runs its own client clock |
| F22 Desired arrival times | Partial: arrival profile drives peak share and transit schedule delay; no departure-time choice |
| F23 Capital cost | Partial: per-km guideway plus vehicles from sourced figures, `preview_line` for the build preview; no terrain or right-of-way multipliers |
| F24 Construction compression | Partial: lines build for `build_years` / 12 before opening (light rail ~6 game months), old version runs during a rebuild; no road capacity loss during works |
| F25 Operating costs and fares | Partial: per-line op cost from NTD cost per car hour x cars, CapMetro fares, distance fares, farebox, monthly budget settlement in `Ledger` |
| F26 Funding | Partial: federal capital grants with a cost-per-rider gate, operating assistance, local 1% sales tax over the served area; no bonds, state appropriation or bankruptcy |
