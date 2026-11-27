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
| `transit.rs` | Transit platforms and edges, stop coverage, path search (`Searcher`), passenger loading (`LineLoads`) | geo, params, schema, roads (`Route`) |
| `roads.rs` | Road routing for bus, trolleybus and BRT: snap stops to the contracted graph, A* by distance, route cache, street shapes (SIM-8) | geo, graph, schema |
| `pairs.rs` | `Pairs`: the kept zone pairs in compressed rows, with reverse-pair index (F34, SIM-13) | none |
| `model.rs` | `Sim`: demand, skims, mode choice, feedback loop, results | everything above |
| `clock.rs` | The one game clock | none |
| `economy.rs` | Capital cost, federal grant gate, `Bond` amortization, `Ledger` with monthly settlement and the bankruptcy rule | model (`LineResult`), params |
| `synth.rs` | Synthetic Travis-like scenario and straight test lines | geo, schema |

Rule of thumb: `graph.rs` and `transit.rs` know nothing about demand or choice;
`model.rs` is the only place that combines them. A new network type (bike
network, airport links) goes in its own module with the same shape: build,
search per origin, return per-destination components.

## Data held by `Sim`

All zone-pair data is indexed by **pair** (`Pairs`, SIM-13): origin `i` owns
pairs `pairs.row(i)`, pair `k` goes to zone `pairs.dest[k]`, and
`pairs.rev[k]` is the reverse pair. Without `Params::strategic_max_min` every
pair is kept, in row-major order, so pair `k = i * Z + j` and the model is
the dense one. With a cutoff, only pairs within that many free-flow car
minutes (both directions, intrazonal always) are kept. Per-origin rows
(`Row`, `OriginOut`, `RowInputs`) are row-aligned: entry `q` is pair
`row(i).start + q`. The transit search still sees dense per-origin car rows
(`dense_row`), since park-and-ride legs need car times to any zone.

| Field | Shape | Built | Rebuilt when |
|---|---|---|---|
| `graph` | CSR road graph | `new` | never (roads are fixed until road building lands) |
| `zpts`, `intra_km`, `vot_min` | per zone | `new` | never |
| `hbw` | per pair: daily work trips | `new` | never |
| `hbo` | per pair: daily other trips | `new` (free-flow car times) | never |
| `car_free` | per-pair skim | `new` | never |
| `car_peak`, `link_time`, `link_vol` | skim / per edge | `equilibrate` | each game day (`equilibrate_warm`) |
| `net` (TransitNet) | platforms, edges, coverage | `new` | every line edit |
| `tsk_walk`, `tsk_drive` | per-pair transit components | every line edit and `equilibrate` | |
| `results`, `transit_od`, `zone_trips` | | every state change | |
| `ledger`, `capital_paid` | books; capital paid per line slot | `new` | capital on `add_line`/`update_line`; settlement at month boundaries |
| `lines`, `pending` | per line slot: version in service; version under construction and its opening day | `new` | line edits; `tick` moves finished `pending` into `lines` |

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

add_line / update_line (construction)  (40-80 ms)
  evaluate the new version in its slot (as below) -> charge capital on that forecast
  -> put the slot back -> store in pending with its opening day

add_line / update_line (instant_build or service change) / remove_line  (20-40 ms)
  rebuild transit net -> transit skims -> finish        (car times held; results.car_times_stale = true)
  add/update then: charge capital above what the line already paid, less federal grant

preview_line  (40-80 ms)
  push empty slot -> evaluate the line in it, read result and capital, restore -> pop slot

tick / skip_to_next_month
  clock -> at a day boundary: open finished pending lines -> equilibrate_warm
  -> ledger.settle once per month boundary crossed

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
  setting); `Sim::opening_day` applies them to a mode's `build_years`.
  Demand, costs and capacities are never compressed.

The clock lives on `Sim` as a public field so saves can serialize it
(`Clock: Serialize`) and the app can read the time of day for lighting and
agents.

## Threading and performance

- **Graph simplification (SIM-2).** `RoadGraph::build` drops dead-end trees that
  hold no zone node (no shortest path between zone nodes can enter one), then
  merges chains through nodes with exactly two neighbors over two links into one
  edge. A chain keeps its links as `members`; its BPR time is the sum of its
  members' BPR times at the shared volume, so congested times are exact, not
  approximated. `link_flows` expands chains back to links and reports pruned
  links at zero volume. On real Travis this halves the nodes Dijkstra touches.

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

### Statewide scale (F34 plan)

Measured with `examples/scale.rs` (synthetic grid region, 1 km road spacing,
default params, one cold equilibrium day, 4 cores):

| Zones | Road nodes | Day | Peak memory |
|---|---|---|---|
| 676 (Travis block groups) | 2,809 | 0.5 s | 0.15 GB |
| 1,600 | 6,561 | 2.6 s | 0.83 GB |
| 3,136 | 12,769 | 11.0 s | 3.1 GB |
| 6,900 (Texas tracts), extrapolated | ~28k | ~60 s | ~15 GB |

Dense storage costs ~316 bytes per zone pair (car skims, four transit skim
rows of `TOd`, demand and flow matrices), and the day grows about as zones²
(trees x destinations). At 6,900 tracts both the 20 s target and a desktop's
memory fail, so the strategic tier changes storage, not the model:

1. **Sparse OD.** Each origin keeps only destinations within a car-time
   cutoff (`strategic_max_min`, ~90 min) plus every LODES commute pair, in
   compressed rows. Gravity distribution, mode choice and loading run over
   that set. Longer trips are intercity travel and belong to F37. Expected
   ~300 destinations per tract: ~2M pairs, under 1 GB.
2. **Bounded trees.** Dijkstra stops at the cutoff instead of settling every
   zone node, so tree cost scales with the destinations kept, not with Texas.
3. **Transit skims only where transit reaches.** Most of Texas has no line;
   transit rows exist only for origins with a stop within access range.
4. **Highway graph.** The statewide build carries motorway to secondary roads
   plus zone connectors (pipeline), contracted at load as today (SIM-2).

Order: (A) sparse OD and bounded trees behind the same `Sim` API, with a test
that results match the dense model when the cutoff covers every pair;
(B) transit rows only where transit reaches; (C) the statewide tract build
and a CI bench against the 20 s target. The API does not change.

**(A) is built (SIM-13).** `Params::strategic_max_min` (None by default, so
Travis runs dense and unchanged) keeps pairs within that free-flow car time.
The pair set comes from free-flow trees bounded at the cutoff; after that
every tree runs until its origin's own kept destinations are settled
(`Tree::set_targets`), so a kept pair is drivable both ways even when road
times are asymmetric (a 10x-cutoff bound is only a guard). Trees reset only
the nodes the last run touched, so a bounded search costs what it explores,
not the size of the statewide graph. Commutes
beyond it are counted in `Results::beyond_cutoff_trips` and still reduce
their home zone's non-work trip budget, so trips are conserved. Measured with
`CUTOFF=30 examples/scale.rs`:

| Zones | Dense | 30-min cutoff |
|---|---|---|
| 676 | 0.5 s, 0.18 GB | 0.4 s, 0.10 GB |
| 3,136 | 12.3 s, 3.25 GB | 3.5 s, 0.52 GB |
| 6,889 | ~60 s, ~15 GB (extrapolated) | 7.2 s, 1.14 GB |

The synthetic region is 166 km across at 6,889 zones and 28k road nodes;
the real statewide graph is larger, so the 20 s check waits for (C).

## Testing

| Test | Covers |
|---|---|
| `tests/schema.rs` | Fixture parses with defaults, round trip, validation errors (dangling ids, NaN, zero lanes, self-loops) |
| `tests/model.rs` | Car-dominated baseline, trip conservation, line adds ridership and shifts share, fare and headway elasticities have the right sign, stable ids on remove, transfers used, congestion feedback, calibration hits targets, tick re-equilibrates, results serialize, tiny fixture runs, preview charges nothing and matches build, extensions pay the difference, monthly settlement, lines open after compressed construction, service changes apply at once while realignments wait |
| `tests/figures.rs` | Sourced defaults (trip rate, NTD cost per car hour, CapMetro fares, walk/wait weights, capital per mile and vehicle prices, sales tax rate, taxable sales per capita, operating assistance share, bond rate and term) equal their rows in `crates/data-pipeline/reference/figures.csv` (D25, D29) |
| `graph.rs` contraction tests | Zone-to-zone times unchanged by pruning and contraction (synthetic always; real build with `TRANSPO_SCENARIO=... cargo test --release -- --ignored`) |
| unit tests in `graph.rs`, `transit.rs`, `geo.rs`, `clock.rs`, `model.rs`, `economy.rs` | Dijkstra and loading, coverage shapes, distance, clock rates, logit, grant gate and settlement arithmetic |
| `examples/bench.rs` | Travis-sized timing (synthetic, or a real `scenario.json`); asserts a line edit < 2 s |
| `examples/redline.rs` | F19 probe: Red Line from real stops on a real build, boardings per stop |

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
