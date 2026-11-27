# sim-core architecture

How the crate is put together, what runs when, and where to change things. The
math is in [MODEL.md](MODEL.md); the API and schema are in the
[README](../README.md).

## Modules

| File | Responsibility | Depends on |
|---|---|---|
| `schema.rs` | Input types (serde). The contract with `data-pipeline`. | serde |
| `lib.rs` | Re-exports, `SimError`, `ScenarioInput::validate` | all |
| `params.rs` | `Params` (every tunable number), per-mode defaults, road lane capacity | schema |
| `geo.rs` | Equirectangular distance and offsets | none |
| `graph.rs` | Road graph (CSR), Dijkstra tree, distance/toll along the tree, all-or-nothing loading | params, schema |
| `transit.rs` | Transit platforms and edges, stop coverage, path search (`Searcher`), passenger loading (`LineLoads`) | geo, params, schema |
| `model.rs` | `Sim`: demand, skims, mode choice, feedback loop, results | everything above |
| `clock.rs` | The one game clock | none |
| `economy.rs` | Capital cost, federal grant gate, `Ledger` with monthly settlement | model (`LineResult`), params |
| `synth.rs` | Synthetic Travis-like scenario and straight test lines | geo, schema |

Rule of thumb: `graph.rs` and `transit.rs` know nothing about demand or choice;
`model.rs` is the only place that combines them. A new network type (bike
network, airport links) goes in its own module with the same shape: build,
search per origin, return per-destination components.

## Data held by `Sim`

All zone-pair data is dense row-major `Z x Z` (`index = i * Z + j`).

| Field | Shape | Built | Rebuilt when |
|---|---|---|---|
| `graph` | CSR road graph | `new` | never (roads are fixed until road building lands) |
| `zpts`, `intra_km`, `vot_min` | per zone | `new` | never |
| `hbw` | Z x Z daily work trips | `new` | never |
| `hbo` | Z x Z daily other trips | `new` (free-flow car times) | never |
| `car_free` | Z x Z skim | `new` | never |
| `car_peak`, `link_time`, `link_vol` | skim / per edge | `equilibrate` | each game day (`equilibrate_warm`) |
| `net` (TransitNet) | platforms, edges, coverage | `new` | every line edit |
| `tsk_walk`, `tsk_drive` | Z x Z transit components | every line edit and `equilibrate` | |
| `results`, `transit_od`, `zone_trips` | | every state change | |
| `ledger`, `capital_paid` | books; capital paid per line slot | `new` | capital on `add_line`/`update_line`; settlement at month boundaries |

## What is recomputed when

```
Sim::new
  validate -> road graph -> zone sizes, VOT -> work trips
  -> free-flow car skim -> gravity (other trips) -> transit net -> equilibrate

equilibrate  (cold, from free flow; Sim::new and after params changes)
  repeat feedback_iterations [4]:  pass(assign) -> MSA average (step 1/n) -> BPR times
  pass -> finish

equilibrate_warm  (once per game day via tick; keeps yesterday's link volumes)
  repeat daily_iterations [1]:     pass(assign) -> MSA average (step 1/(4+n)) -> BPR times
  pass -> finish

pass  (one parallel loop over origins, ONE shortest-path tree each)
  tree -> car skim row -> transit skim row -> mode choice row -> [AON load of the row's car demand on the same tree]

add_line / update_line / remove_line  (20-40 ms)
  rebuild transit net -> transit skims -> finish        (car times held; results.car_times_stale = true)
  add/update then: charge capital above what the line already paid, less federal grant

preview_line  (40-80 ms)
  push line -> as add_line (no charge) -> read result and capital -> pop line -> as remove_line

tick / skip_to_next_month
  clock -> equilibrate_warm at a day boundary -> ledger.settle once per month boundary crossed

finish
  mode choice -> transit loading -> line economics -> Results
```

Why line edits skip the road side: a new line changes car volumes by a few
percent at most, and the player wants instant feedback while drawing. Car times
catch up at the next day boundary, which is how the doc's strategic tier works
("once per in-game day, incremental"). `results.car_times_stale` tells the UI
the road numbers are from before the edit.

Cost of a line edit: transit path search is over platforms only (hundreds), not
the road graph, and origins with no stop in reach skip it entirely. Mode choice
is Z x Z x 2 purposes x 4 segments of a 6-way logit, parallel by origin.

## The clock

`Clock` ([src/clock.rs](../src/clock.rs)) is the only time source.

- 1 real second = 1 game minute at 1x (`GAME_SECONDS_PER_REAL_SECOND = 60`), so a
  game day is 24 real minutes and the AM peak hour plays out in one real minute.
- `timelapse` multiplies that (UI offers 2x, 10x, 60x). At 60x a game day is 24
  real seconds, which is the budget for `equilibrate` statewide in M3.
- Zoom never touches the clock. The app changes what it draws, not how fast
  time runs.
- `advance(real_dt)` returns day boundaries crossed; `Sim::tick` re-equilibrates
  once if any were crossed (not once per day: at extreme time-lapse it would
  only redo the same calculation).
- `skip_to_next_month()` jumps to day 30n, 00:00 (budgets settle monthly, 30-day
  months).
- New scenarios start at 05:00 on day 0, just before the AM peak.
- `Params::construction_speedup [12]` and `Params::build_days(real_days)`
  compress build and permitting times (the doc's 12x default, 1x to 24x
  setting). Demand, costs and capacities are never compressed.

The clock lives on `Sim` as a public field so saves can serialize it
(`Clock: Serialize`) and the app can read the time of day for lighting and
agents.

## Threading and performance

- Rayon parallelizes every per-origin loop (car skims, car assignment, transit
  skims, transit loading, mode choice). Each worker reuses its own Dijkstra or
  transit-search buffers (`map_init` / `fold`), so the hot loops do not allocate.
- **Fused pass.** Mode choice for origin `i` needs only row `i` of the car and
  transit skims, and that row's car demand can be loaded on the tree that just
  produced the skim. So each feedback round builds one tree per origin and does
  skim, transit search, mode choice and loading in one go, instead of a skim
  pass and a separate assignment pass. This halved the trees per equilibrium.
- **Warm daily start.** `equilibrate_warm` keeps the previous day's link
  volumes and treats them as `feedback_iterations` rounds of MSA history, so one
  round plus a final pass holds the equilibrium (a test checks speeds, vehicle-km
  and drive share stay within 1–3% of a cold run).
- Dijkstra stops once every zone's road node is settled. The heap key packs the
  cost (as f32 bits) and node into one `u64`, the settled flag guards against
  stale entries, and distances accumulate in f64 so they do not drift. Edge
  targets live in a dense `to` array next to the times the inner loop reads.
- Car assignment pushes demand up the shortest-path tree in reverse settle order
  (one pass per origin) instead of tracing each destination's path.
- `Sim` is `Send` and has no interior mutability. The app holds it in a
  `Mutex`; long calls (`tick`, `equilibrate`) belong on a worker thread.

Scaling notes for M3 (statewide): dense Z x Z storage is fine to ~2,000 zones.
At 6,900 tracts each matrix is ~380 MB as f64, so the strategic tier will need
sparse demand, f32 or blocked skims, and a contracted highway graph. The
module boundaries above are where those changes go; the API does not change.

## Testing

| Test | Covers |
|---|---|
| `tests/schema.rs` | Fixture parses with defaults, round trip, validation errors (dangling ids, NaN, zero lanes, self-loops) |
| `tests/model.rs` | Car-dominated baseline, trip conservation, line adds ridership and shifts share, fare and headway elasticities have the right sign, stable ids on remove, transfers used, congestion feedback, calibration hits targets, tick re-equilibrates, results serialize, tiny fixture runs, preview charges nothing and matches build, extensions pay the difference, monthly settlement |
| `tests/figures.rs` | Sourced defaults (trip rate, NTD cost per car hour, CapMetro fares, walk/wait weights, capital per mile and vehicle prices) equal their rows in `crates/data-pipeline/reference/figures.csv` (D25, D29) |
| unit tests in `graph.rs`, `transit.rs`, `geo.rs`, `clock.rs`, `model.rs`, `economy.rs` | Dijkstra and loading, coverage shapes, distance, clock rates, logit, grant gate and settlement arithmetic |
| `examples/bench.rs` | Travis-sized timing; asserts a line edit < 2 s |

Tests use `SynthConfig::small()` (64 zones) so the suite runs in under a second
in debug builds.

## Extending

- **New transit mode:** add a `Mode` variant in `schema.rs` and a row in
  `params::mode_defaults`. Nothing else changes.
- **New sourced default:** add the figure to the pipeline's `figures.csv`
  (data thread), copy it into `Params` or `mode_defaults` with a comment, and
  add a `check` line in `tests/figures.rs`.
- **New zone attribute from the pipeline:** add an optional field with a
  `#[serde(default)]` in `schema.rs`, document it in the README schema section,
  use it in `model.rs`.
- **New choice (e.g. scooter):** extend `N_CHOICES`, the index constants and
  `asc` in `params.rs`, add its cost in `Sim::costs`, add the field to
  `ModeSplit`.
- **Road building:** make `graph` rebuildable (it is built from
  `ScenarioInput`), then call `equilibrate`. Free-flow skims and other-trip
  distribution would need refreshing too.
