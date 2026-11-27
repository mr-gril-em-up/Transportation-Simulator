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
