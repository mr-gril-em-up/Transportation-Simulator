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

// Draw a line: returns a stable id. Capital is charged now. By default the
// line is under construction until sim.opening_day(mode) (light rail: 6 real
// years / 12 = ~183 game days), listed in results().pending_lines, then carries
// riders. Set params.instant_build = true to open lines at once (tests, probes).
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
| `add_line(TransitLine) -> LineId` | 40-80 ms (20-40 ms with `instant_build`) | Adds a line and charges its capital (less any federal grant on its forecast). It enters service on `opening_day(mode)`; until then it is in `pending_lines()` with no riders. With `params.instant_build` (default false, SIM-4) it opens at once and `results()` is refreshed. Ids are never reused. |
| `opening_day(Mode) -> u64` | free | Game day a line drawn today would open: `build_years` for the mode / `construction_speedup`. |
| `pending_lines() -> Vec<PendingLine>` | free | Lines (or new versions) under construction: `id, name, mode, opens_day, replaces_open_line`. Also in `results().pending_lines`. |
| `update_line(LineId, TransitLine)` | 20-80 ms | Replaces a line (same id). Service changes (headway, fare, speed, span, name) apply at once, also to a running version during a rebuild; extra vehicles from a shorter headway are charged. Moving, adding or removing stops, or changing mode, is construction: the extra capital is charged now, the old version keeps running, the new one opens on `opening_day`. |
| `remove_line(LineId)` | 20-40 ms | Removes a line, open or under construction. Its id is retired. No refund. |
| `preview_line(TransitLine) -> LinePreview` | 40-80 ms | Build preview: ridership (`line: LineResult`), `capital_cost`, `federal_grant`, `capital_per_daily_boarding`. Evaluates the line, then removes it; ledger and ids unchanged. |
| `ledger() -> &Ledger` | free | Transit authority books: `cash`, `current` month, settled `history` (`MonthStatement`), `served_population`, outstanding `bonds`, `debt()`, `distressed_months`, `bankrupt_month`. `add_line`/`update_line` charge capital (net of any federal grant); month boundaries settle fares, operating cost, assistance, local tax and bond payments, then apply the bankruptcy rule. |
| `issue_bond(principal) -> Result<Bond>` | free | Borrows now at `bond_rate` (5.43%, Bond Buyer Revenue Bond Index) over `bond_term_months()` (360 by default). Cash arrives at once, level payments start next month, or once the lines under construction at issue open (interest is capitalized until then, SIM-9). Errors on a non-positive amount, above `bond_capacity()`, or after bankruptcy. 12 distressed months in a row (debt service above operating surplus, each bond counted only once the lines building at its issue have opened; or cash below zero) set `ledger().bankrupt_month` (F26, SIM-5). |
| `bond_capacity() -> f64`, `tax_capacity() -> f64` | free | Borrowing cap (additional-bonds test, SIM-7, SIM-9): principal a new bond may still borrow, from annual tax capacity (residents within `tax_area_km` 3 km of any stop on lines open or under construction x taxable sales x 1%) / `bond_coverage_min` (2.0, Houston METRO's covenant) less current debt service. 0 with no lines. For the app's bond menu. |
| `skip_to_next_month() -> u64` | ~2 s | Jumps the clock to next month, re-runs the day, settles the month. Use this, not `sim.clock.skip_to_next_month()`, so the ledger settles. |
| `lines() -> Vec<(LineId, &TransitLine)>` | free | Every line as last drawn (the version under construction if any). `results().lines` has only lines in service. |
| `results() -> &Results` | free | Latest results (see below). |
| `zone_results() -> Vec<ZoneResult>` | ms | Per zone: daily trips, transit trips and share, stop coverage. For heatmaps. |
| `link_flows() -> Vec<LinkFlow>` | ms | Per directed road edge: peak-hour volume, v/c, congested speed. For congestion bands. |
| `desire_lines(n) -> Vec<DesireLine>` | ms | Top n zone pairs by daily trips, with transit trips. For the M0 desire-line view. |
| `equilibrate()` | seconds | Full mode choice and congested car assignment from free-flow times. Call after changing `params`. |
| `equilibrate_warm()` | ~2 s at Travis size | Daily re-run starting from the previous day's road volumes (`daily_iterations` rounds). |
| `tick(real_dt) -> u64` | free or ~2 s | Advances `sim.clock`; at a day boundary opens lines whose construction finished, runs `equilibrate_warm` once, and settles the ledger at month boundaries. Call from a worker thread. |
| `tick_budgeted(real_dt, budget) -> u64` | about `budget` | Same day, spread over calls (SIM-18): a boundary opens due lines and starts the day's re-run as a job, and each call runs at most about `budget` of it, so line edits go in between. Results and car times stay the previous day's until the job ends; months settle when it does. Statewide with a 100 ms budget: the longest call is 0.5 s instead of a 14 s block. Use instead of `tick`, not alongside it. |
| `Sim::new_warm(input, params, &RoadState)`, `road_state() -> RoadState`, `started_warm()` | | Warm start (SIM-19): save `road_state()` (road volumes plus a fingerprint of the road graph, ~3.5 MB JSON statewide) when a game ends and pass it to `new_warm` next time; the cold equilibrium becomes one warm day. Statewide: 21 s instead of 71 s, starting at 40 km/h instead of 33. A state from another road network is ignored (`started_warm()` is false and the start is cold). |
| `day_progress() -> Option<f64>` | free | Share of the running day job done (0 to 1), None when none runs. For a "new day computing" indicator. |
| `calibrate_mode_shares(ModeSplit, iters)` | ~0.1 s/iter | Fits the logit constants so commute trips reproduce target shares (ACS journey to work); transit as one share unless `transit_drive` has its own target (SIM-10). |
| `commute_mode_shares() -> ModeSplit` | ~0.3 s | Commute (home-work) shares under current constants: the quantity the fit targets. |
| `input() -> &ScenarioInput` | free | The scenario as loaded (lines live in `lines()`). |
| `sim.params`, `sim.clock` | | Public fields. Change params then call `equilibrate()`. |

`Results` (refreshed by every call that changes state):

| Field | Meaning |
|---|---|
| `daily_trips`, `work_trips`, `other_trips` | Person trips per day by choice (`ModeSplit`: drive, carpool, transit_walk, transit_drive, walk, bike) |
| `mode_share` | `daily_trips` as shares |
| `unserved_trips` | Trips with no available option (no car, no transit, too far to walk) |
| `beyond_cutoff_trips` | Daily work trips between zones beyond `Params::strategic_max_min`, counted but not modeled (0 without a cutoff, SIM-13) |
| `lines: Vec<LineResult>` | One per active line, in id order |
| `peak_vehicle_km`, `peak_avg_speed_kph` | Road network health in the AM peak hour |
| `work_peak_hour_share` | Share of commutes arriving in the busiest hour |
| `car_times_stale` | True after a line edit until the next `equilibrate` or `equilibrate_warm` (car congestion not yet updated) |
| `eval_ms` | Wall time of the call that produced these results |

`LineResult`: `id, name, mode, length_km, run_min, daily_boardings,
boardings_by_stop` (aligned with the line's `stops`), `passenger_km,
max_segment_load, peak_hour_load, peak_hour_capacity, vehicles_required,
revenue_hours_per_day, operating_cost_per_day, fare_revenue_per_day,
farebox_recovery, segment_loads` (`[forward, reverse]` per stop pair),
`segment_shapes` (bus, trolleybus and BRT: the street route per stop pair as
`[lon, lat]` points from stop to stop; an empty entry means a straight
fallback segment; empty for other modes, SIM-8). `LinePreview::line` carries
it too, so a draft snaps to streets while it is drawn.

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

`TransitLine`: `{ "name"?, "mode", "stops": [{ "lon", "lat", "name"?, "park_ride"?, "park_ride_spaces"? }],
"headway_min", "offpeak_headway_min"?, "fare"?, "fare_per_km"?, "speed_kph"?, "span_hours"? }`.
`park_ride_spaces` (optional; omitted = unlimited) caps cars parked a day at a
`park_ride` stop: a full lot gets a shadow-price access penalty until the cars
fit (SIM-11), and `LineResult.park_ride_cars_by_stop` reports use per stop.
`headway_min` is the peak headway (work trips); `offpeak_headway_min`
(default: the same) is what non-work trips see and sets off-peak revenue hours; the fleet covers the more frequent of the two. Omitted
values take per-mode defaults ([src/params.rs](src/params.rs) `mode_defaults`).
Modes: `bus, trolleybus, brt, tram, light_rail, commuter_rail, metro,
high_speed_rail, monorail, people_mover, gondola, maglev, ferry`. Hyperloop is
deliberately absent.

`ScenarioInput::validate()` checks ids, ranges and NaN, self-loop links, and
zero-lane links without an explicit capacity; the pipeline should run it on its
output. A small working example is [tests/fixtures/tiny.json](tests/fixtures/tiny.json).

M0/M1 zones are Travis County block groups. The pipeline now prunes and
contracts the road graph itself (DATA-2); sim-core also drops dead ends and
contracts degree-2 chains at load (SIM-2). The two compose exactly, so a
scenario from either an old or a new pipeline build gives the same results.

### Performance on the real Travis build (4-thread container, release build)

`cargo run --release -p sim-core --example bench path/to/scenario.json` on the
published `data-48453-2023` build (766 zones, 93,461 road nodes, 121,120 links,
148,257 commute pairs; fetch it with the pipeline's `scripts/download-build.sh`).

| Operation | Before SIM-2 | With dead-end pruning and chain contraction |
|---|---|---|
| `Sim::new` (baseline to equilibrium) | 10.3 s | 5.8 s |
| `add_line` / `preview_line` | 0.04 s / 0.09 s | 0.04 s / 0.10 s |
| `equilibrate_warm` (each game day) | 3.5 s | 1.9 s |
| `equilibrate` (full re-run) | 8.8 s | 4.8 s |

Contraction leaves 45,746 of 93,406 connected nodes and 125,474 of 212,360
directed edges. Mode shares and line ridership are identical to 1e-12, and an
ignored test (`contraction_keeps_zone_times_on_real_scenario`) checks
zone-to-zone times on 40 origins.

### Performance on synthetic regions (4-thread container, release build)

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
Contracting degree-2 road nodes (SIM-2, above) roughly halved the real
Travis numbers again. The next lever is renumbering the surviving nodes
densely so Dijkstra's arrays shrink with the graph.

Scaling with zone count (`cargo run --release -p sim-core --example scale --
26 40 56`): 676 zones 0.5 s / 0.15 GB, 1,600 zones 2.6 s / 0.83 GB, 3,136
zones 11.0 s / 3.1 GB per cold day. Dense zone-pair storage extrapolates to
~60 s and ~15 GB at Texas's 6,900 tracts. With `Params::strategic_max_min`
(SIM-13) only pairs within that free-flow car time are kept: `CUTOFF=30` runs
3,136 zones in 2.7 s / 0.52 GB and 6,889 zones in 6.2 s / 1.12 GB. On the real
statewide build (6,896 tracts, 214k road nodes) a game day takes 13.8 s at a
60-minute cutoff (`examples/statewide.rs`), under the F34 target of 20 s.
Road assignment steps by a Frank-Wolfe line search (SIM-15): with 8 cold
rounds (69 s) the statewide peak average is 33 km/h at start and 38 km/h by
day 3 (plain MSA gave 3 km/h). A statewide line edit (preview or add, today's
61 CapMetro lines loaded) takes 0.78 s, at 1.4 GB peak (SIM-17). With all
455 Texas lines it took 8.2 s; a spatial index for the stop and tract joins
brings it to 4.9 s (SIM-20), still over the 2 s target. Re-searching and re-loading only the transit
networks an edit can reach brings it to about 1.0 s (SIM-21). Plan and numbers: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#statewide-scale-f34-plan).

Line edits are fast because they leave car travel times alone and only redo
transit paths, mode choice and transit loading. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#what-is-recomputed-when).
### Calibration: ACS commute shares with today's network (F14)

`examples/calibrate.rs` loads today's CapMetro network from `transit.json`
(74 route refs) into the real Travis build, fits the logit constants to ACS
commute shares and reports the fit, bus and Red Line boardings:

```
cargo run --release -p sim-core --example calibrate -- \
  ../data-pipeline/data/out/48453-2023/scenario.json \
  ../data-pipeline/data/out/48453-2023/existing_lines.json
```

| | Commute drive / carpool / transit / walk / bike | Buses/day | Red Line/day |
|---|---|---|---|
| Old defaults | 76.0 / 18.5 / 1.2 / 2.0 / 2.3 | 28,292 | 6,373 |
| Fitted, SIM-10 (all 10 stations park-and-ride) | 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | 65,518 | 1,917 |
| Fitted, SIM-11 (3 real lots with spaces) | 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | 65,953 | 1,471 |
| Fitted on `existing_lines.json` with GTFS headways (now the defaults) | 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | 77,802 | 1,367 |
| Same constants after the line-searched assignment (SIM-15; a refit moves them under 0.02, not adopted) | 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | 77,349 | 1,352 |
| Same constants on the DATA-9 republish (stops from GTFS, Red Line coded commuter rail; a refit moves them under 0.01, not adopted) | 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | 77,062 | 1,317 |
| Target / count | ACS 83.5 / 10.4 / 2.3 / 2.9 / 0.8 | ~69-70k (NTD weekday) | 1,500 (APTA weekday) |

Shares are % of commuters who travel (ACS less work from home and other).
Park-and-ride runs only at the Red Line's three real lots (Leander 619,
Lakeline 935, Howard 301 spaces); they hold 277 / 37 / 3 cars, so the space
limit does not bind today. The drive-access constant (3.76 below walk access,
inherited from SIM-10's tuned 3% split) is still unsourced and sets the Red
Line figure; see [MODEL.md section 6](docs/MODEL.md#6-mode-choice).
It takes about 22 s (7 s load, 11 s fit).

### Starting with today's network

`ScenarioInput.lines` are existing service: open on day 0, no capital
charge, counted in the tax area. The pipeline writes today's CapMetro network
to `existing_lines.json` (DATA-7), and the shell puts it into `lines` when the
game starts with it. Its budget on real Travis, per year at the defaults:

| In | | Out | |
|---|---|---|---|
| Local sales tax (977,834 residents within 3 km x $39,100 x 1%, SIM-12) | $382M | Bus operating cost (5,539 revenue hours a weekday at $182) | $368M |
| Fares | $37M | Red Line operating cost | $19M |
| Operating assistance (14.3%) | $55M | | |
| **Total** | **$475M** | **Total** | **$387M** |

That is an $88M a year surplus, and $2.8B of borrowing capacity. Real CapMetro collected $382M of
sales tax in FY2023, which is what the tax rate is calibrated to; operating
cost runs ~40% high because weekday service is billed every day (MODEL.md
known gap 6).

### Calibration probe: CapMetro Red Line (F19)

**Update (SIM-10, SIM-11):** with the fitted constants the Red Line is probed
inside today's network by `examples/calibrate.rs`: 1,471/day with park-and-ride
at its three real lots (2% low; 1,917 when all ten stations had lots). The
drive-access constant was tuned in SIM-10, so this is still not an independent
check. Run alone, as below, it now carries 861/day, because
without bus feeders and with walk access fitted to all of CapMetro it loses
riders. The rest of this section records the pre-calibration probe.

`examples/redline.rs` builds the Red Line from the real stop positions in the
pipeline's `transit.json` (OSM route 550, 10 stations Downtown to Leander, all
with park-and-ride), adds it to the real Travis build as commuter rail, and
prints daily boardings per stop:

```
cargo run --release -p sim-core --example redline -- \
  ../data-pipeline/data/out/48453-2023/scenario.json \
  ../data-pipeline/data/out/48453-2023/transit.json 30
```

First results, default `Params` (logit constants not yet calibrated to ACS):

| Uniform headway | Model boardings/day | vs 1,500 |
|---|---|---|
| 30 min | 2,285 | +52% |
| 32 min (real peak) | 2,198 | +47% |
| 40 min | 1,883 | +26% |
| 52 min (real service average) | 1,495 | 0% |
| 60 min | 1,282 | -15% |

With separate peak and off-peak headways (SIM-3), the replica at the real
32-minute peak and 51-minute midday headway gives **1,738 boardings/day**:

```
cargo run --release -p sim-core --example redline -- scenario.json transit.json 32 51
```

The reference is APTA's 2023 average weekday, 1,500 boardings
(`ridership_capmetro_rail_avg_weekday`; NTD's annual 477,322 includes Saturday
and event service, so dividing it by days overstates weekdays). Headways come
from `headway_capmetro_rail_peak_min` (32) and `..._midday_min` (51), span
`span_capmetro_rail_weekday_hours` (15.1). **1,738 is 16% high, inside the F19
30% band**, before any ACS calibration of the logit constants. The table above
uses one headway for both purposes and brackets the answer.

Caveats:
- Leander and Lakeline sit in Williamson County, whose residents are not zones
  in the Travis build, so the north end is undercounted (24 and 28 boardings).
- Nearly all riders arrive by park-and-ride (`transit_drive` 0.07% of trips vs
  `transit_walk` 0.006%); the walk side needs checking against stop-level counts.
- Each purpose sees one headway all day (SIM-3); there is no hour-by-hour
  service profile.

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
  hour, CapMetro fares, the 2x walk and wait weights, capital and vehicle
  prices, the funding keys, and the bond rate, term and coverage ratio (D25, D29).
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
| F14 Logit mode split | Done for Travis: constants fitted to ACS 2019-2023 commute shares with today's CapMetro network (SIM-10); other metros are M2+ |
| F15 Sim API | sim-core side done; Tauri commands are the app's |
| F18 Incremental re-evaluation | Done on the synthetic Travis-sized region (20-40 ms); confirm on real Travis data |
| F21 One clock | Partial: `Clock` with time-lapse and skip-to-next-month in sim-core; app still runs its own client clock |
| F22 Desired arrival times | Partial: arrival profile drives peak share and transit schedule delay; no departure-time choice |
| F23 Capital cost | Partial: per-km guideway plus vehicles from sourced figures, `preview_line` for the build preview; no terrain or right-of-way multipliers |
| F24 Construction compression | Partial: lines build for `build_years` / 12 before opening (light rail ~6 game months), old version runs during a rebuild; no road capacity loss during works |
| F25 Operating costs and fares | Partial: per-line op cost from NTD cost per car hour x cars, CapMetro fares, distance fares, farebox, monthly budget settlement in `Ledger` |
| F26 Funding | Partial: federal capital grants with a cost-per-rider gate, operating assistance, local 1% sales tax over the tax area (zones within 3 km of a stop), 30-year bonds with capitalized interest during construction, a borrowing cap and the bankruptcy rule; no state appropriation (Texas gives none) |
