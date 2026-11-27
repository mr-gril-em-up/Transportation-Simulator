# data-pipeline

Turns public Census and OpenStreetMap data into the input files Transpo-Sim's simulation core loads: demand zones, home-to-work commute flows, the drivable road graph, and existing transit routes. Delivers roadmap features F02 and F03, and holds the sourced reference figures that F23/F25 (costs, fares) will use.

Travis County (Austin) is the M0/M1 test bench. The same binary builds all of Texas with `--area 48`; nothing in the code is Travis-specific except the default argument.

## Run

```sh
cd crates/data-pipeline
./scripts/fetch.sh                                   # ~800 MB of raw inputs into data/raw (gitignored)
cargo run --release                                  # Travis County -> data/out/travis
cargo run --release -- --area 48 --out data/out/texas  # all of Texas
```

### Use a published build (no raw downloads)

Every manual run of the **data-pipeline** workflow publishes its output as GitHub Release `data-<area>-<year>` (DATA-1; prerelease, assets replaced on re-run). To use the latest Travis build in the app or sim-core:

```sh
cd crates/data-pipeline
eval "$(./scripts/download-build.sh)"        # Travis, LODES 2023 -> data/out/48453-2023, sets TRANSPO_SCENARIO
cd ../../app && npm run tauri dev             # the app loads $TRANSPO_SCENARIO (app decision A14)
```

`AREA=48 YEAR=2023 ./scripts/download-build.sh` fetches a statewide build once one is published. The release also carries `manifest-<area>-<year>.json` for a quick look without the archive.

### Publish a new build

Actions tab → **data-pipeline** → **Run workflow** (area `48453`, year `2023`). It downloads the raw data, builds, uploads a run artifact, and publishes or updates the release.

## Test

```sh
cargo test -p data-pipeline --locked
cargo clippy -p data-pipeline --all-targets --locked -- -D warnings
```

Covers a full build on a synthetic Travis County sample in the real file formats (generated in the test, no network or downloads), CenPop parsing, OSM speed and distance helpers, and the reference-figures CSV. Details and the checklist for a real build: [docs/TESTING.md](docs/TESTING.md).

## Interfaces

### CLI

```
tsim-data [--raw DIR] [--out DIR] [--area 48453[,48491...]] [--year 2023] [--margin 0.05] [--level bg|tract|county]
```

| Flag | Default | Meaning |
|---|---|---|
| `--raw` | `data/raw` | Directory holding the files `scripts/fetch.sh` downloads |
| `--out` | `data/out/travis` | Where output files are written (created if missing) |
| `--area` | `48453` | Comma-separated GEOID prefixes. `48453` = Travis County, `48453,48491` = Travis + Williamson, `48` = Texas |
| `--year` | `2023` | LODES data year, and the end year of the ACS 5-year estimates (must match the downloaded file names) |
| `--level` | `bg` | Zone level of the outputs: `bg` (block group), `tract` or `county`. Coarser levels merge block groups after ACS and land area are filled (DATA-5). Texas has 18,638 block groups, about 6,900 tracts and 254 counties |
| `--roads` | `all` | `highway` keeps motorway, trunk, primary and secondary roads and their ramps only (DATA-8), for the statewide tier. Releases get a `-highway` tag suffix |
| `--margin` | `0.05` | Degrees of padding around the zones' bounding box when clipping OSM (about 5 km) |

The build prints `manifest.json` to stdout and the elapsed time to stderr.

### Output files

| File | Consumer | Contents |
|---|---|---|
| `scenario.json` | sim-core | `sim_core::ScenarioInput`: zones, road nodes, road links, commutes. Validated with `ScenarioInput::validate()` before it is written. |
| `zones.json` | app map, debugging | Per-zone detail: GEOID, resident workers, in/out commuters, and the ACS/TIGER values written to the scenario |
| `roads.json` | app map | Road graph with OSM class, street name and drawing geometry (`shape`) |
| `transit.json` | calibration, "keep existing transit" toggle | Existing OSM transit routes with ordered, named stops |
| `manifest.json` | humans, CI | Counts, totals, bounding box, and the top 20 commute flows (the M0 exit check) |

Ids are positions: zone `id` in `scenario.json` is its index in `zones.json`, and road node `id` is its index in `roads.json` `nodes`. Full field reference: [docs/OUTPUT_FORMAT.md](docs/OUTPUT_FORMAT.md).

## Design and rationale

- **Rust, not Python** (D14): builds `sim_core::ScenarioInput` directly, so a schema change breaks this compile instead of the game at load time.
- **Block-group zones from Centers of Population** (D13): one small CSV gives population and a population-weighted centroid, with no geometry library.
- **Routing simplification** (DATA-2): `scenario.json` drops road fragments, prunes dead ends without a zone and merges identical pass-through links. Paths between zones are unchanged; on Travis it cuts road nodes 44%. `roads.json` keeps full detail for drawing.
- **ACS income and zero-car share at block-group level** (DATA-4): B19013 and B25044 from the ACS table-based summary files (no API key); suppressed estimates take the tract, then county, household-weighted mean. Land area from TIGER `ALAND`. Both inputs are optional.
- **Today's network as existing lines** (DATA-7, DATA-9): `existing_lines.json` holds one sim-core line per GTFS route of every agency feed in `reference/gtfs_feeds.csv` (CapMetro, DART, Houston METRO, Trinity Metro, DCTA, VIA, CCRTA, El Paso County, Sun Metro), stops and frequencies from GTFS, so calibration and a later "start with today's network" setting use the same network. `--existing-lines` also puts them in `scenario.json` `lines`; it is off by default because sim-core loads `lines` with no switch.
- **GTFS headways** (DATA-6): each feed is reduced to weekday peak and midday headway, span and the longest trip's stops per route. CapMetro routes in `transit.json` also carry weekday peak and midday headways and span from the GTFS feed, matched by route number.
- **Commute mode shares** from ACS B08301, summed over the area, go in the manifest for mode-choice calibration (F14).
- **TxDOT traffic counts** (DATA-10): `counts.json` puts every TxDOT AADT count station that matches a nearby scenario link by route number or street name (within 100 m) on that link, with the opposite carriageway on divided roads, for car-assignment calibration (F38).
- **Highway-only roads** (DATA-8): `--roads highway` drops roads below secondary, so a statewide tract build has a road graph small enough for the strategic tier (F34).
- **Tract and county zone levels** (DATA-5): `--level tract|county` merges block groups by GEOID prefix after the block-group build. Counts add; centroid, income and zero-car share are population-weighted; flows inside one coarse zone become intrazonal. The statewide scenario at block-group level is far too large for a sub-20 s day, so coarser levels come first.
- **Zone connectors** (D19) go to the nearest local-road node (never motorway or trunk) in the largest connected component, so access is not free and clipping fragments cannot strand a zone.
- **Lanes per direction in `scenario.json`**, because sim-core capacity is per direction; `roads.json` keeps OSM's total for drawing.
- **Road class** (`RoadLink.class`, optional, added by sim-core for F38): the raw OSM `highway` value (`motorway`, `motorway_link`, `primary`, ...), filled for every link. sim-core derives capacity from it when `capacity_vph` is omitted. Contraction keeps class boundaries, so a ramp never merges into the freeway.
- **No starting transit** in `scenario.json` (design doc "Starting map"); existing routes stay in `transit.json` for calibration.

Full step-by-step reasoning: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Every default and its source: [docs/ASSUMPTIONS.md](docs/ASSUMPTIONS.md).

## Documentation

| Doc | Read it for |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the pipeline works, step by step, and why it is built this way |
| [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md) | Every input dataset: publisher, URL, vintage, license, known gaps, citation |
| [docs/OUTPUT_FORMAT.md](docs/OUTPUT_FORMAT.md) | Every output file and field, units, id rules, how it maps to sim-core's schema |
| [docs/ASSUMPTIONS.md](docs/ASSUMPTIONS.md) | Every default and simplification in the code, with the reason and the fix path |
| [docs/TESTING.md](docs/TESTING.md) | What the tests cover, the synthetic fixture, and how to check a real build |
| [docs/REFERENCE_FIGURES.md](docs/REFERENCE_FIGURES.md) | Sourced costs, fares, operating costs, ridership and zone counts that replace the design doc's placeholders ([`reference/figures.csv`](reference/figures.csv)) |

## Code map

| File | Role |
|---|---|
| `src/main.rs` | CLI argument parsing, calls `build` |
| `src/lib.rs` | `build()`: orchestrates the steps, writes files and the manifest |
| `src/aggregate.rs` | Merges block-group zones and flows into tracts or counties (`--level`) |
| `src/acs.rs` | Reads ACS summary-file tables (pipe-delimited) and TIGER `ALAND` (dBASE), fills zone income, zero-car share and area with tract/county fallback |
| `src/census.rs` | Reads Centers of Population (zones) and LODES RAC/WAC/OD, aggregates blocks to block groups |
| `src/lines.rs` | Turns every feed's GTFS routes into sim-core `TransitLine`s (existing service), with timetable overrides and park-and-ride stops from `figures.csv` |
| `src/counts.rs` | Reads TxDOT AADT stations and snaps them to scenario links by distance and route number or street name (`counts.json`) |
| `src/gtfs.rs` | Lists the agency feeds (`reference/gtfs_feeds.csv`), reduces each timetable to weekday peak and midday headway, span and stop sequence per route, and attaches CapMetro's to OSM routes by `ref` |
| `src/osm.rs` | Two-pass OSM PBF reader: road graph and transit routes, clipped to a bbox |
| `src/scenario.rs` | Converts the layers to `sim_core::ScenarioInput`, picks each zone's road node |
| `src/model.rs` | Serde types for the detail files (`zones.json`, `roads.json`, `transit.json`) |
| `scripts/fetch.sh` | Downloads the raw inputs |
| `scripts/download-build.sh` | Downloads a published build from GitHub Releases and prints the `TRANSPO_SCENARIO` export |
| `tests/sample.rs` | End-to-end build on a synthetic Travis sample |
| `tests/reference.rs` | Checks `reference/figures.csv` is well formed and cited |
| `reference/figures.csv` | Sourced reference figures with citations |
| `tests/support/pbf.rs` | Tiny OSM PBF writer so the OSM fixture is code, not a binary file |
| `../../.github/workflows/data-pipeline.yml` | PR tests + manual real-data build |

## Known gaps

- No block-group polygons (TIGER): planned F31.
- No synthetic population: F04. No non-work trips: F13.
- No GTFS schedules for the Red Line replica: F19.
- Out-of-state commuters (LODES `od_aux`) not loaded.
- JSON output will be large statewide; binary package is F33.
- sim-core's Travis model load (baseline) is about 6 s on the simplified graph. Line edits (about 25 ms) and daily re-runs (under 2 s) already meet their targets; residential streets stay in for M1 (DATA-3).
- Reference figures still missing: primary-source value of travel time, express-lane tolls, Texas road costs, 2024-USD normalization ([docs/REFERENCE_FIGURES.md](docs/REFERENCE_FIGURES.md)).

## Status

- F02 done (#3). F03 done (#3).
- Real-data Travis build: published as release `data-48453-2023` by the Actions workflow; checks in [docs/TESTING.md](docs/TESTING.md).
- F31 partial: statewide `--area 48` builds and publishes (release `data-48-2023`); ACS income and zero-car share plus TIGER land area added. Tract and county zone levels (`--level`, DATA-5) added; releases for them are tagged `data-<area>-<year>-<level>`. Polygons are next.
- F04, F32, F33 not started.
