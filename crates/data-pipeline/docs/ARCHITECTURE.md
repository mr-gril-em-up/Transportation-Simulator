# Data pipeline architecture

## Purpose

Transpo-Sim plays on the real map of Texas. Every resident, job and road the simulation uses comes from public data, processed offline into files the game loads. This crate is that offline step. It runs at build time on a developer machine or in CI, never inside the game.

Two goals shaped it:

1. **Prototype speed on Travis County.** M0 needs real zones, real commute flows and real roads for one county as fast as possible, so the demand model can be tuned against Austin.
2. **No rewrite for Texas.** The same code, unchanged, must build all 254 counties for M3. Area selection is a command-line argument, and every input file already covers the whole state.

## Why Rust, not the Python the design doc proposed

The overview doc suggested a Python pipeline (osmnx, geopandas). The team chose a Rust crate inside the Cargo workspace instead:

- **One toolchain.** The sim core and app shell are Rust. Contributors and CI need only `cargo`.
- **Direct use of sim-core's schema.** The pipeline depends on `sim-core` and builds `sim_core::ScenarioInput` values, then calls `validate()`. A schema change breaks the pipeline's compile, not the game at load time.
- **Statewide speed.** The Texas OSM extract is several hundred MB. A streaming PBF reader in Rust handles it in a couple of passes without a geospatial Python stack.
- **Few dependencies.** `csv`, `flate2`, `osmpbf`, `serde`, `anyhow`. No GDAL, no shapefile reader: inputs were chosen to avoid geometry libraries (see Zones below).

The cost: Python would make ad-hoc analysis easier. Analysis can still read the JSON outputs from any language.

## Pipeline steps

`build()` in `src/lib.rs` runs these in order.

```
CenPop2020_Mean_BG48.txt ──► 1. zones ──┐
tx_rac_*.csv.gz ─────────► 2. workers ─┤
tx_wac_*.csv.gz ─────────► 3. jobs ────┤
tx_od_main_*.csv.gz ─────► 4. flows ───┼──► 6. scenario ──► validate ──► scenario.json
texas-latest.osm.pbf ────► 5. roads,   │                                 zones.json
                              transit ─┘                                 roads.json, transit.json
                                                                         manifest.json
```

### 1. Zones (`census::read_zones`)

A zone is a 2020 census block group (12-digit GEOID: state 2, county 3, tract 6, block group 1). The design doc assigns block groups to City zoom and the M0/M1 demand model, and Texas has about 18,600 of them, small enough to run statewide later.

The source is the Census Bureau's 2020 **Centers of Population** file for block groups. One CSV row per block group gives the population and the population-weighted centroid. This one file replaces two heavier steps (reading TIGER polygons and computing centroids from block-level population), and a population-weighted centroid is the better point for loading trips: it sits where people live, not in the middle of an empty field that happens to be inside the boundary.

Rows are filtered by GEOID prefix (`--area`) and sorted by GEOID so zone indexes are identical across runs.

### 2–3. Resident workers and jobs (`census::add_lodes_totals`)

LODES RAC (Residence Area Characteristics) counts workers by home block; WAC (Workplace Area Characteristics) counts jobs by work block. The pipeline sums column `C000` (all jobs) into each block's block group. LODES 8 is keyed to 2020 census blocks, and a 2020 block's first 12 digits are its 2020 block group, so the roll-up is a string prefix with no crosswalk file.

### 4. Commute flows (`census::read_flows`)

LODES OD gives worker counts per (home block, work block) pair, column `S000` (all jobs). The pipeline aggregates to block-group pairs:

- Both ends in the study area: summed into a flow (`Commute` in the scenario).
- Home in the area, work outside it (but in Texas): added to the home zone's `commute_out`.
- Work in the area, home outside it: added to the work zone's `commute_in`.
- Neither end in the area: skipped.

Intra-zone flows (home and work in the same block group) are kept; sim-core decides how to treat them.

Only the OD **main** file is read: both ends in Texas. Out-of-state residents working in Texas (OD **aux**) are a rounding error for Travis and are listed as a gap.

### 5. Roads and transit (`osm::read_osm`)

OpenStreetMap is read straight from the Geofabrik Texas `.osm.pbf` in two streaming passes, so memory scales with the roads kept, not the whole file:

- **Pass 1, ways and relations.** Keep ways whose `highway` tag is a drivable class (motorway through living_street; service roads, tracks and paths are dropped to keep the graph small). Record each way's node list, class, speed, lanes, one-way flag and name, and count how many times each node is used. Keep `type=route` relations for bus, trolleybus, tram, light rail, train, subway and monorail, with their stop nodes in order (role `stop*`, falling back to `platform*` when a route maps only platforms).
- **Pass 2, nodes.** Store coordinates only for nodes pass 1 asked for, plus names for stop nodes.

Then `build_roads` splits each way at every node shared with another way (and at its endpoints). Each piece becomes one edge with its haversine length and the intermediate points as `shape`. A piece is kept only if every point lies inside the clip box, so the clipped graph never has an edge that leaves the area. The clip box is the zones' bounding box padded by `--margin` degrees, which keeps the roads just past the county line that commuters use.

Transit routes are kept if at least one stop is inside the clip box, and keep their full stop list (a commuter rail line that starts outside the county is still one line).

Speed, lane and one-way rules are in [ASSUMPTIONS.md](ASSUMPTIONS.md).

If `texas-latest.osm.pbf` is missing, this step is skipped with a note, and the scenario gets one placeholder road node so it still validates. That mode exists for quick census-only checks; real builds always include roads.

### 6. Scenario (`scenario::to_scenario`)

Converts the layers into `sim_core::ScenarioInput`:

- Zone `id` = index in the sorted zone list. Road node `id` = index in `roads.nodes`, so scenario ids line up with the detail files. Zone ids are dense; road node ids are a subset after simplification (below).
- Each zone's `road_node` is the nearest node to its centroid that touches a **local** road (anything except motorway and trunk classes and their links). Loading trips onto a freeway node would let them skip the local network and make access look free. Candidates are also limited to the largest connected component of the road graph: clipping at the margin leaves small fragments, and a zone loaded onto one would pass validation but reach nothing. Search uses a 0.01° grid and expanding rings, so statewide matching is fast.
- Road lanes are converted from OSM's total-lane count to sim-core's lanes per direction.
- Zero-length links and self-loops are dropped (sim-core's validator rejects them).
- **Routing simplification** (`scenario::simplify`, DATA-2). sim-core's run time scales with road nodes, and most OSM nodes carry no routing choice. Three steps, none of which changes any zone-to-zone path or its length:
  1. Drop links outside the largest connected component (clipping fragments no zone uses).
  2. Prune dead ends repeatedly: a node with a single neighbor that is not a zone connector can be neither origin, destination nor a through point. This removes cul-de-sacs, driveways and freeway stubs cut by the clip box.
  3. Contract pass-through nodes: a non-connector node with exactly two links of equal speed, lanes per direction and direction (two-way, or one-way flowing through) becomes one link of the summed length. Links that differ are left alone, so speed and capacity changes stay where OSM puts them.

  Kept nodes keep their roads.json ids (sim-core allows sparse ids). On the real Travis build this takes road nodes from 93,461 to 52,210 and links from 121,120 to 79,583, and sim-core's baseline from 12.0 s to 6.8 s with identical mode shares and peak speed. `roads.json` keeps every node and edge for drawing.
- `lines` is empty: the game starts with no transit by default (design doc, "Starting map"). Existing routes stay in `transit.json`.

`ScenarioInput::validate()` runs before anything is written to `scenario.json`; a failure aborts the build.

### Manifest

`manifest.json` records the area, LODES year, sources, counts, totals and the top 20 flows by GEOID. The M0 exit test is "top 20 commute flows match LODES totals": compare that table against LODES directly (see [TESTING.md](TESTING.md)).

## Determinism

Same inputs and arguments produce byte-identical outputs: zones sorted by GEOID, flows sorted by (home, work), routes sorted by OSM id, no timestamps. Road node order follows the order ways appear in the PBF, which is stable for a given extract.

## Scaling to Texas

Nothing changes but `--area 48`. Expected sizes: about 18,600 zones, a few hundred thousand flows, and a road graph in the low millions of edges. The JSON outputs will be hundreds of MB at that size; switching to a binary encoding is planned once sim-core has a loader that needs it. Memory is dominated by the node coordinate map in `osm.rs`.

## Extending

- **New zone attribute** (for example ACS `zero_car_share`): add a reader in `census.rs`, a field on `model::Zone`, and set it in `scenario.rs`. Download it in `fetch.sh`. Document the source in DATA_SOURCES.md.
- **New road class or tag rule**: `class_defaults` and the tag parsing in `osm.rs`.
- **Polygons for map fills**: add a TIGER block-group reader and write a GeoJSON beside `zones.json`; keep it out of `scenario.json`.
- **Another state**: the code is state-agnostic except the file names in `lib.rs` and `fetch.sh` (`48`, `tx`, `texas-latest`). Parameterize those when a second state is needed.
