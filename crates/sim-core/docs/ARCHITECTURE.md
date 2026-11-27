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
| `geo.rs` | Equirectangular distance and offsets, `PointGrid` radius and nearest-point queries (SIM-20) | none |
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
| `tsk_walk`, `tsk_drive` (+ `_off`) | per origin, a row-aligned vector of transit components, **empty when no destination has transit** (`TSkim`, SIM-17) | every line edit and `equilibrate` | |
| `idle_rows`, `idle_key` | per origin: mode totals of an origin with no transit, keyed on the params | each `finish` | dropped when car times move (`pass`) or params change |
| `results`, `transit_od`, `zone_trips` | | every state change | |
| `ledger`, `capital_paid` | books; capital paid per line slot | `new` | capital on `add_line`/`update_line`; settlement at month boundaries |
| `lines`, `pending` | per line slot: version in service; version under construction and its opening day | `new` | line edits; `tick` moves finished `pending` into `lines` |

## What is recomputed when

```
Sim::new
  validate -> road graph -> zone sizes, VOT -> work trips
  -> free-flow car skim -> gravity (other trips) -> transit net -> equilibrate

equilibrate  (cold, from free flow; Sim::new and after params changes)
  repeat feedback_iterations [4]:  pass(assign) -> step (1 first, then Frank-Wolfe line search) -> BPR times
  pass -> finish

equilibrate_warm  (once per game day via tick; keeps yesterday's link volumes)
  repeat daily_iterations [1]:     pass(assign) -> Frank-Wolfe step (floor 0.1/(4+n)) -> BPR times
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
  push empty slot -> evaluate the line in it, read result and capital -> put the
  previous transit state back (moved, not recomputed, SIM-17) -> pop slot

tick / skip_to_next_month
  clock -> at a day boundary: open finished pending lines -> equilibrate_warm
  -> ledger.settle once per month boundary crossed

tick_budgeted (SIM-18)  (each call about `budget`)
  clock -> at a day boundary: open finished pending lines (re-evaluate transit at once)
  -> start a DayJob; each call runs part of it:
     round 0: park-and-ride prices -> rounds 1..daily_iterations: pass_range over slices of
     origins, summing their AON loads -> Frank-Wolfe step -> final round: pass_range slices
     -> commit car (and transit) skims -> finish -> settle months
  line edits between calls run as usual on the previous day's car times; if one
  lands during the final round, transit is re-skimmed at commit. One landing
  during an assign round mixes old- and new-network slices into that round's
  load; later days pull it back, as they do any day's error. Note that
  the budget is a floor per slice, not a hard cap: the final commit and
  finish are one step (0.5-0.6 s statewide)

finish
  mode choice (origins with no transit reuse their totals, SIM-17) -> transit loading
  -> line economics -> Results
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
  volumes, so one line-searched round (SIM-15) plus a final pass holds the
  equilibrium (a test checks speeds, vehicle-km
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
The pair set comes from free-flow trees bounded at the cutoff, and a pair
is kept only if **both** directions are within it (SIM-14), so every kept
pair is drivable both ways. After that every tree runs until its origin's own
kept destinations are settled (`Tree::set_targets`); the bound (1.01x the
cutoff at free flow, 4x when congested) is only a guard. SIM-13 first kept a
pair if either direction was within the cutoff: on the real statewide graph,
with one-way ramps, some destinations were unreachable from the other side,
and those trees searched all of Texas (45 s a day). Trees reset only
the nodes the last run touched, so a bounded search costs what it explores,
not the size of the statewide graph. Commutes
beyond it are counted in `Results::beyond_cutoff_trips` and still reduce
their home zone's non-work trip budget, so trips are conserved. Measured with
`CUTOFF=30 examples/scale.rs`:

| Zones | Dense | 30-min cutoff |
|---|---|---|
| 676 | 0.5 s, 0.18 GB | 0.4 s, 0.10 GB |
| 3,136 | 12.3 s, 3.25 GB | 3.5 s, 0.52 GB |
| 6,889 | ~60 s, ~15 GB (extrapolated) | 6.2 s, 1.12 GB |

The synthetic region is 166 km across at 6,889 zones and 28k road nodes.

**(C) Real Texas** (`examples/statewide.rs` on release
`data-48-2023-tract-highway`: 6,896 tracts, 213,801 road nodes, 283,966
links, 3.88M LODES flows; 4 cores):

| Cutoff | Cold start (`Sim::new`) | Game day (`equilibrate_warm`) | Peak memory | Work trips beyond cutoff |
|---|---|---|---|---|
| 45 min | 35 s | 10.3 s | 1.8 GB | 6.1M |
| 60 min | 45 s | **13.8 s** | 2.2 GB | 5.2M |
| 90 min | 74 s | 24.8 s | 2.9 GB | 4.4M |

60 minutes meets the 20 s target (F34). Each feedback pass is one tree per
tract, so cost grows with the area a cutoff covers.

**Congestion and cold-start rounds (SIM-15).** The first measurement gave a
peak average of 3 to 7 km/h. The cause was not the demand (peak-hour
vehicle-km per resident is 5.4 statewide vs 3.7 on Travis, about 1.4x a
real Texas figure) but the assignment: four rounds of MSA left a quarter of
the free-flow load on the freeways, 33,000 veh/h on a 6-lane US 75 segment,
and 92% of vehicle-hours sat on links above twice capacity. With the
Frank-Wolfe line search, at 60 min (`examples/statewide.rs <scenario> 60
<rounds> <days>`):

| Cold rounds (`feedback_iterations`) | Cold start | Peak speed cold | Day 1 | Day 3 | Day 8 |
|---|---|---|---|---|---|
| 4 (MSA, before) | 45 s | 3 km/h | 7 km/h | | |
| 4 | 42 s | 12 km/h | 18 km/h | 29 km/h | 39 km/h |
| 6 | 53 s | 24 km/h | 29 km/h | 35 km/h | |
| 8 | 69 s | 33 km/h | 35 km/h | 38 km/h | |

A game day stays 13.4 to 14.5 s.

**The congested tree guard (SIM-16).** Peak trees stop at 4x the cutoff
(240 min at 60). SIM-15 first reported that this never binds, but that
check (`car_unreachable_pairs`) reads the free-flow skim. In the jammed
first days 1,700 to 5,700 kept pairs ran past the guard and lost the car, and
with no walk, bike or transit at those distances their trips showed as
unserved: 76k to 130k a day. A pair past the guard now keeps the car at
the guard time, with free-flow distance and toll; those trips are not loaded
on the roads (a few thousand of 34M kept pairs). Unserved trips are now 0,
and a game day is unchanged. Removing the guard also gives 0, but the deep
searches take a game day to 25 s. `car_past_guard_pairs()` counts them, and
`examples/statewide.rs` prints it daily. Statewide games should start with 8 cold rounds, behind a
loading state or a cached equilibrium; the default stays 4 so Travis is
unchanged. **Open:** about 39 km/h after a week is still low for a peak
hour. Two known causes, for F38 with TxDOT counts: roads at 55 to 80 km/h get
900 veh/h per lane (`lane_capacity_vph`, set for signalized arterials), which
also applies to rural two-lane highways that carry ~1,700; and BPR puts no cap
on delay at very high volume/capacity.

**Line edits (SIM-17).** With today's 61 CapMetro lines loaded
(`LINES=existing_lines.json examples/statewide.rs`), a statewide
`preview_line` took 2.5 s and `add_line` 2.5 s here (the app measured about
4.5 s on its machine), against F18's 2 s. Two causes:

- A preview, or an add of a line still under construction, evaluated the
  network twice, once with the line and once to put it back. The previous
  transit state is now moved aside and moved back (`TransitState`), not
  recomputed.
- Transit skims were stored for every kept pair (44 bytes each, both access
  modes), though ~85% of tracts have no transit. Each origin now stores an
  empty row when nothing is reachable (`TSkim`), the search returns early when
  no platform is within access, and an origin with no transit reuses its mode
  totals from the last full choice while car times and params are unchanged
  (`idle_rows`).

| | Preview | Add | Peak memory |
|---|---|---|---|
| Before | 2.5 s | 2.5 s | 2.2 GB |
| After | **0.78 s** | **0.79 s** | **1.4 GB** |

Travis results are unchanged (calibration, Red Line 1,352/day, buses
77,349/day; `bench` preview 0.036 s, add 0.039 s). A game day is unchanged
(14 s).

**Day rollover (SIM-18).** `tick` runs the whole day under the caller's
lock, so the app paused edits for about 20 s at every statewide day.
`tick_budgeted` runs it as a `DayJob` in slices of origins
(`rayon` threads x 32 per slice) until the call's budget is used. The pass is
split into `pass_range` (reads state, changes nothing) and `commit_outs`, so
the blocking path and the job share the code. Statewide with a 100 ms
budget (`examples/statewide.rs`): the day takes 17.0 s of compute over 85
calls (vs 14.3 s blocking; slices balance cores less well), and the longest
call is 0.50 s (the final commit). Results are the previous day's until the
job ends; a test checks the budgeted day lands within 1e-6 of the blocking
one, and that a line added mid-day is in the day's results.

**Warm start (SIM-19).** The statewide cold start is 8 feedback rounds from
free flow (71 s). `Sim::road_state()` returns the road volumes with a
fingerprint of the contracted road graph (FNV-1a over every member's ends,
free-flow time and capacity, so it is stable across Rust versions);
`Sim::new_warm` loads them and runs one warm game day instead
(`equilibrate_warm`). Statewide after 3 game days saved: **21.4 s** to start,
peak 40 km/h (cold: 70.9 s, 33 km/h). A state whose fingerprint or length
does not match is ignored, so a rebuilt scenario starts cold. The app can
ship a state with each scenario release, or save one when a game closes.
The fingerprint covers the road graph only, not demand: a state saved under
other params, mode constants, existing lines or a recalibrated release on the
same roads is accepted. It is a starting guess, not a checked result: the
warm day's Frank-Wolfe step moves it toward the new demand, so a mismatched
save costs accuracy on day 1, not correctness.

**Transit network build at statewide scale (SIM-20).** With every Texas
agency's GTFS loaded (455 lines, 19.5k stops) a line edit took 8.2 s, and
4.5 s of it was `TransitNet::build` comparing every stop with every other
stop (transfers) and with every tract (access, egress, nearest zone).
`geo::PointGrid` buckets points by lon/lat cell; `near` returns every point
that can lie within a radius, sorted by index, and the caller keeps its exact
distance test, so the lists and their order are the same as the all-pairs
scans. The longitude span is widened by the smallest cosine inside the
query's latitude band, so no point within the radius is missed. `nearest`
grows the radius until the best hit lies inside it. `Searcher` also stops
clearing every platform and zone before each search: it resets only what the
last run touched, and sorts the touched platforms before egress so ties
resolve in the same order. Statewide, 455 lines, 4 cores:

| | Network rebuild | Line edit (preview / add) | Longest budgeted-day call |
|---|---|---|---|
| Before | 4.5 s | 8.2 s / 8.1 s | 3.7 s |
| After | **0.15 s** | **4.9 s / 4.6 s** | 3.1 s |

Travis results are bit-identical (`DUMP=` output of `examples/statewide.rs`
with one rayon thread; with more threads parallel float sums already differ
in the last bit between runs). What is left of the edit: transit skims
1.8 s, ridership loading 1.8 s (both re-search every origin with transit),
and 0.75 s flattening per-pair vectors; the longest day call is the same
loading in the day's final slice. Next: re-search and re-load only the
transit networks an edit can reach.

**Edits scoped to the networks they reach (SIM-21).** A transit search
only moves along ride and transfer edges, so it never leaves the connected
components of the platforms it starts from: the origin's walk-access
platforms and the park-and-ride platforms within the drive limit.
`TransitNet::comp` numbers the components and `comp_sigs` hashes what a
search inside each one reads (line ids, stop numbers, positions, service,
fares, park-and-ride and its penalty), keyed by line id and stop number so
a component untouched by an edit hashes the same in the rebuilt net. On an
edit, `Sim::edit_scope` compares the old and new nets:

1. Components whose hash is new (in the new net) or gone (from the old
   one) are changed.
2. An origin is re-searched if it seeds a changed component in either net;
   every other origin keeps its skim rows, with park-and-ride platform ids
   renumbered when a removal shifted them.
3. Components seeded by a re-searched origin have new loads. Origins that
   seed one are re-loaded, and every other line keeps its stored load
   (`Sim::loads`, carried by line id). A line no re-searched origin
   reaches has riders whose choices did not change, so its load is the same.

Mode choice still runs for every origin with transit (0.2 s). The scope
falls back to a full recompute when off-peak skims switch on or off. A
preview saves the stored loads with the rest of the transit state and puts
them back. Statewide, 455 lines, 4 cores, editing a copy of an Austin bus
line:

| | Line edit (preview / add) | Longest budgeted-day call |
|---|---|---|
| SIM-20 | 4.9 s / 4.6 s | 3.1 s |
| SIM-21 | **0.95 s / 1.07 s** | 2.5 s |

With one rayon thread the results after the edit are bit-identical to
SIM-20's full recompute, statewide and on Travis (`DUMP=` md5; with one
thread the statewide edit is 1.8 s against 13.5 s). On Travis all of
CapMetro is one component, so a Travis edit is still a full recompute. A unit test
(`scoped_line_edits_match_a_full_recompute`) runs adds, a crossing line, a
headway change, a removal that renumbers platforms and a preview on two
networks, and after each one redoes the skims and loading in full: skims
must match exactly, loads to 1e-9. Breaking any of the four rules above
(drive seeds, old-net seeds, the load carry, the renumbering) fails it. The
day's longest call is still the full loading in its last slice; the day
changes car times, so nothing carries over there.

**Ridership loading in slices (SIM-22).** With 455 lines the day's final
call did all of the ridership loading at once: 2.5 s, of which 2.1 s was
`load_transit` re-running every transit origin's search. `finish` is now
three steps: `demand` (mode choice to per-pair transit demand, park-and-ride
cars), `load_demand` over a range of origins, and `finish_results`. The day
job adds a loading phase after the final pass that loads one slice of
origins per step and sums the loads, so the work spreads over calls like the
passes do. While it runs, the stored per-line loads are cleared, because
they no longer match the committed car times; an edit in that window
therefore recomputes in full (`edit_scope` sees no stored loads), and its
results stand: the job ends without overwriting them. A preview is not an
edit: it puts `lines_version` back with the rest of the transit state, so
the job carries on. Statewide, 455 lines,
100 ms budget: longest call **0.45 s** (was 2.5 s), the day 22.9 s over
106 calls (was 23.0 s over 85). `day_progress` counts the loading as one
more pass.

## Testing

| Test | Covers |
|---|---|
| `tests/schema.rs` | Fixture parses with defaults, round trip, validation errors (dangling ids, NaN, zero lanes, self-loops) |
| `tests/model.rs` | Car-dominated baseline, trip conservation, line adds ridership and shifts share, fare and headway elasticities have the right sign, stable ids on remove, transfers used, congestion feedback, line-searched assignment unjams in two rounds where MSA does not (SIM-15), jammed pairs past the tree guard keep the car (SIM-16), preview puts results back bit for bit and edits after a params change re-choose origins without transit (SIM-17), budgeted day matches the blocking one and keeps a mid-day edit (SIM-18), an edit while the day loads ridership matches the same edit after a blocking day and a preview then leaves the day alone (SIM-22), warm start from saved roads matches and rejects another graph (SIM-19), calibration hits targets, tick re-equilibrates, results serialize, tiny fixture runs, preview charges nothing and matches build, extensions pay the difference, monthly settlement, lines open after compressed construction, service changes apply at once while realignments wait |
| `tests/figures.rs` | Sourced defaults (trip rate, NTD cost per car hour, CapMetro fares, walk/wait weights, capital per mile and vehicle prices, sales tax rate, taxable sales per capita, operating assistance share, bond rate and term) equal their rows in `crates/data-pipeline/reference/figures.csv` (D25, D29) |
| `graph.rs` contraction tests | Zone-to-zone times unchanged by pruning and contraction (synthetic always; real build with `TRANSPO_SCENARIO=... cargo test --release -- --ignored`) |
| unit tests in `graph.rs`, `transit.rs`, `geo.rs`, `clock.rs`, `model.rs`, `economy.rs` | Dijkstra and loading, coverage shapes, distance, `PointGrid` matches brute force, scoped line edits match a full recompute (SIM-21), clock rates, logit, grant gate and settlement arithmetic |
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
