# Testing

## Run

```sh
cd crates/data-pipeline
cargo test
cargo clippy --all-targets -- -D warnings
```

CI runs both on every PR that touches this crate (`.github/workflows/data-pipeline.yml`, job `test`). No network or downloaded data is needed.

## What is covered

| Test | File | Checks |
|---|---|---|
| `builds_travis_sample` | `tests/sample.rs` | Full `build()` on the synthetic sample: zone filtering and GEOID order; block-to-block-group roll-up of RAC, WAC and OD; `commute_in`/`commute_out`; road splitting at shared nodes; clipping (a road to Dallas and a footway are dropped); `maxspeed` in mph; motorway one-way and tagged lanes; transit stop order and names; a route outside the area dropped; `scenario.json` loads with `ScenarioInput::from_json` and passes `validate()`; zones never connect to a freeway-only node or to a disconnected fragment closer to the centroid; per-direction lane conversion; empty `lines` |
| `whole_state_area_keeps_both_counties` | `tests/sample.rs` | `--area 48` keeps both counties; flows between them become internal; census-only build (no PBF) still validates |
| `zones_without_header_row` | `src/census.rs` | Centers of Population file without a header |
| `speeds`, `distance` | `src/osm.rs` | `maxspeed` parsing; haversine length |

## The synthetic Travis sample

`tests/sample.rs` `write_sample` writes, into a temp directory, small files in the **real formats**:

- A Centers of Population CSV with a byte-order mark: four Travis block groups downtown and one in Williamson County.
- Gzipped LODES RAC, WAC and OD files with 15-digit block codes, including flows that cross the county line and one with neither end in the area.
- An OSM PBF built in code by `tests/support/pbf.rs`: six road nodes around Congress Avenue, a freeway, a road to Dallas (outside the clip box), a footway (not drivable), a CapMetro bus route with two named stops, and a train route in Dallas.

The numbers and coordinates are made up but placed in real Austin locations. The PBF writer exists so the fixture is readable code instead of an opaque binary file.

## Checking a real build

After `./scripts/fetch.sh && cargo run --release` (or the Actions workflow), open `data/out/travis/manifest.json` and check:

1. `zones` is roughly 700–800 for Travis County (Travis had 1.29M people in 2020; block groups average about 1,500 people).
2. `population` matches the 2020 census Travis County total (1,290,188).
3. `jobs` and `resident_workers` are in the high hundreds of thousands.
4. **M0 exit test:** pick the first few `top_flows` and confirm the counts against OnTheMap (https://onthemap.ces.census.gov) or by summing the raw OD file for those block-group pairs. They must match exactly; the pipeline only sums.
5. `road_edges` is in the tens of thousands and `road_km` in the low thousands.
6. `transit_routes` includes the CapMetro Red Line (`mode` `train`) and MetroRapid routes; grep `transit.json` for `"Red Line"`.

`scenario.json` is validated during the build, so a successful run already guarantees referential integrity.

## First real build (2026-11-27)

Actions run [45932728431](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45932728431), Travis County, LODES 2023, Geofabrik extract of that day. Download 695 MB; build 58 s on a GitHub runner; `validate()` passed.

| Manifest field | Value | Check |
|---|---|---|
| `zones` | 766 | Plausible for 1.29M people |
| `population` | 1,290,188 | Exact match to the 2020 census Travis County total |
| `resident_workers` / `jobs` | 646,869 / 947,590 | Travis is a net job importer |
| `internal_commuters` | 442,116 | 68% of resident workers stay in the county |
| `commute_out` / `commute_in` | 197,805 / 498,758 | Large inflow from Williamson, Hays and Bastrop |
| `flows` | 148,257 block-group pairs | |
| `road_nodes` / `road_edges` / `road_km` | 93,461 / 121,367 / 18,530 km | The rectangular clip box includes parts of neighboring counties, so road km exceeds Travis alone |
| `transit_routes` | 158 | Red Line present: OSM route `550` (CapMetro), both directions, 10 stops each; also four Amtrak Texas Eagle variants |

Largest flow: 183 workers from block group 484530003081 to 484530004013 (downtown). Not yet done: comparing the top 20 against OnTheMap independently (F07).

Second run (Actions run 45935018791, from the release-publishing branch) produced the same totals and published release `data-48453-2023`. Loading its `scenario.json` into `sim_core::Sim::from_json` with default `Params` gave a baseline in 23 s on a 4-core cloud container (93k road nodes); drive 76%, carpool 21%, walk and bike 3.5%. That is far above the 2-second line-edit target, mostly from the size of the road graph (see ASSUMPTIONS: residential streets are kept).
