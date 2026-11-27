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

No network where you are? Open the repo's **Actions** tab, pick the **data-pipeline** workflow, click **Run workflow** (area `48453`, year `2023`). It downloads the data, builds, and attaches the output files to the run as a downloadable artifact.

## Test

```sh
cargo test -p data-pipeline --locked
cargo clippy -p data-pipeline --all-targets --locked -- -D warnings
```

Covers a full build on a synthetic Travis County sample in the real file formats (generated in the test, no network or downloads), CenPop parsing, OSM speed and distance helpers, and the reference-figures CSV. Details and the checklist for a real build: [docs/TESTING.md](docs/TESTING.md).

## Interfaces

### CLI

```
tsim-data [--raw DIR] [--out DIR] [--area 48453[,48491...]] [--year 2023] [--margin 0.05]
```

| Flag | Default | Meaning |
|---|---|---|
| `--raw` | `data/raw` | Directory holding the files `scripts/fetch.sh` downloads |
| `--out` | `data/out/travis` | Where output files are written (created if missing) |
| `--area` | `48453` | Comma-separated GEOID prefixes. `48453` = Travis County, `48453,48491` = Travis + Williamson, `48` = Texas |
| `--year` | `2023` | LODES data year (must match the downloaded file names) |
| `--margin` | `0.05` | Degrees of padding around the zones' bounding box when clipping OSM (about 5 km) |

The build prints `manifest.json` to stdout and the elapsed time to stderr.

### Output files

| File | Consumer | Contents |
|---|---|---|
| `scenario.json` | sim-core | `sim_core::ScenarioInput`: zones, road nodes, road links, commutes. Validated with `ScenarioInput::validate()` before it is written. |
| `zones.json` | app map, debugging | Per-zone detail sim-core does not need: GEOID, resident workers, in/out commuters |
| `roads.json` | app map | Road graph with OSM class, street name and drawing geometry (`shape`) |
| `transit.json` | calibration, "keep existing transit" toggle | Existing OSM transit routes with ordered, named stops |
| `manifest.json` | humans, CI | Counts, totals, bounding box, and the top 20 commute flows (the M0 exit check) |

Ids are positions: zone `id` in `scenario.json` is its index in `zones.json`, and road node `id` is its index in `roads.json` `nodes`. Full field reference: [docs/OUTPUT_FORMAT.md](docs/OUTPUT_FORMAT.md).

## Design and rationale

- **Rust, not Python** (D14): builds `sim_core::ScenarioInput` directly, so a schema change breaks this compile instead of the game at load time.
- **Block-group zones from Centers of Population** (D13): one small CSV gives population and a population-weighted centroid, with no geometry library.
- **Zone connectors** go to the nearest local-road node (never motorway or trunk) in the largest connected component, so access is not free and clipping fragments cannot strand a zone.
- **Lanes per direction in `scenario.json`**, because sim-core capacity is per direction; `roads.json` keeps OSM's total for drawing.
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
| `src/census.rs` | Reads Centers of Population (zones) and LODES RAC/WAC/OD, aggregates blocks to block groups |
| `src/osm.rs` | Two-pass OSM PBF reader: road graph and transit routes, clipped to a bbox |
| `src/scenario.rs` | Converts the layers to `sim_core::ScenarioInput`, picks each zone's road node |
| `src/model.rs` | Serde types for the detail files (`zones.json`, `roads.json`, `transit.json`) |
| `scripts/fetch.sh` | Downloads the raw inputs |
| `tests/sample.rs` | End-to-end build on a synthetic Travis sample |
| `tests/reference.rs` | Checks `reference/figures.csv` is well formed and cited |
| `reference/figures.csv` | Sourced reference figures with citations |
| `tests/support/pbf.rs` | Tiny OSM PBF writer so the OSM fixture is code, not a binary file |
| `../../.github/workflows/data-pipeline.yml` | PR tests + manual real-data build |

## Known gaps

- ACS fields (`zero_car_share`, `median_income`) use sim-core defaults: planned with F04/F31.
- No block-group polygons (TIGER): planned F31.
- No synthetic population: F04. No non-work trips: F13.
- No GTFS schedules for the Red Line replica: F19.
- Out-of-state commuters (LODES `od_aux`) not loaded.
- JSON output will be large statewide; binary package is F33.
- Reference figures still missing: dollar value of travel time, toll per mile, DART local fare, Texas road costs, 2024-USD normalization ([docs/REFERENCE_FIGURES.md](docs/REFERENCE_FIGURES.md)).

## Status

- F02 done (#3). F03 done (#3).
- Real-data Travis build: run through the Actions workflow; see [docs/TESTING.md](docs/TESTING.md) for the checks.
- F04, F31, F32, F33 not started.
