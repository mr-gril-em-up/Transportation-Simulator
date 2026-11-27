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
| `builds_travis_sample` | `tests/sample.rs` | Full `build()` on the synthetic sample: zone filtering and GEOID order; block-to-block-group roll-up of RAC, WAC and OD; `commute_in`/`commute_out`; road splitting at shared nodes; clipping (a road to Dallas and a footway are dropped); `maxspeed` in mph; motorway one-way and tagged lanes; transit stop order and names; a route outside the area dropped; `scenario.json` loads with `ScenarioInput::from_json` and passes `validate()`; zones never connect to a freeway-only node or to a disconnected fragment closer to the centroid; per-direction lane conversion; ACS income and zero-car share with tract and county fallback, TIGER `ALAND` from a dBASE file written in the test, both reaching `scenario.json`, manifest counts; routing simplification leaves only the two connectors and the link between them, with matching manifest counts; empty `lines` |
| `whole_state_area_keeps_both_counties` | `tests/sample.rs` | `--area 48` keeps both counties; flows between them become internal; census-only build (no PBF) still validates |
| `county_level_merges_block_groups` | `tests/sample.rs` | `--level county` on the whole sample: 5 block groups become 2 counties, totals unchanged, 5-digit GEOIDs, manifest `zone_level` and `block_groups`, `scenario.json` validates |
| `tracts_merge_counts_weights_and_flows` | `src/aggregate.rs` | Tract merge: sums, population-weighted centroid and income, area only when complete, unpopulated tract, flows summed with intrazonal kept; county merge |
| `commute_shares_sum_listed_block_groups` | `src/acs.rs` | B08301 shares sum only the listed block groups, skip county rows, and add to 1 |
| `red_line_uses_timetable_and_park_and_ride` | `src/lines.rs` | One line per ref (most stops), routes without headways dropped, `train` becomes commuter rail with the figures.csv 32/51 min timetable, Leander and Lakeline stops get park-and-ride and Downtown does not, midday headway carried |
| `headways_from_a_tiny_feed` | `src/gtfs.rs` | Busiest-Wednesday service selection with a calendar_dates removal, weekend service ignored, directions combined per window, peak and midday headways, span, empty windows give `None` |
| `dates_and_weekdays` | `src/gtfs.rs` | GTFS date to day number, Wednesday test, times past 24:00 |
| `acs_parses_and_flags_suppressed` | `src/acs.rs` | Summary-file table parsing (block-group rows only, state row skipped), suppressed sentinels, blanks and zero-household rows become `None` |
| `zones_without_header_row` | `src/census.rs` | Centers of Population file without a header |
| `simplify_contracts_chains_and_prunes_dead_ends` | `src/scenario.rs` | Dead-end pruning, fragment removal, contraction only across equal speed and lanes, one-way contraction only when traffic flows through, summed lengths |
| `speeds`, `distance` | `src/osm.rs` | `maxspeed` parsing, including rejecting 0, negative, NaN, inf and >200 km/h; haversine length |

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

Second run (Actions run 45935018791, from the release-publishing branch) produced the same totals and published release `data-48453-2023`. Loading its `scenario.json` into `sim_core::Sim::from_json` with default `Params` gave a baseline in 12 s on a 4-core cloud container with sim-core as of D28 (23 s before it; 93k road nodes); drive 76%, carpool 21%, walk and bike 3.5%. This is model load (baseline equilibrium), not the 2-second line-edit target: sim-core re-evaluates a line edit in about 25 ms and a daily re-run in under 2 s. Load time scales mostly with the size of the road graph (see ASSUMPTIONS: residential streets are kept; DATA-3).

Routing simplification (DATA-2), checked by re-running `scenario::to_scenario` on that release's `zones.json` and `roads.json`: road nodes 93,461 → 52,210, links 121,120 → 79,583, summed link length 18,483 → 15,410 km (the dropped km are dead ends and fragments). sim-core baseline 11.96 s → 6.79 s, with mode shares and peak average speed equal to the full graph to 12 significant digits. A coarser road class cut would shrink load time further but changes results; DATA-3 keeps residential and unclassified streets for M1.

Re-measured after sim-core's own pruning and contraction (SIM-2, #28) merged: the full graph now baselines in 6.86 s and the DATA-2 graph in 6.17 s, again with identical mode shares and peak speed. SIM-2 captures most of the gain internally; DATA-2 adds about 10% on top and keeps `scenario.json` smaller (fewer links to parse and ship). Same container, same release, main as of #25.

## First statewide build (2026-11-27)

Actions run [45938347124](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45938347124), `--area 48`, LODES 2023, published as release `data-48-2023` (tarball 220 MB). An earlier attempt (run 45936886984) failed `validate()` on a link with a zero or unparseable `maxspeed`; `parse_speed` now rejects those.

Timing on a standard `ubuntu-latest` runner: download 7.5 min, compile plus build 81 s (the build alone is about 40 s), publish 30 s. Memory was not a problem.

| Manifest field | Value | Check |
|---|---|---|
| `zones` | 18,638 | Exact match to the 2020 Texas block group count (`tx_block_groups`) |
| `population` | 29,145,505 | Exact match to the 2020 census Texas total |
| `resident_workers` / `jobs` | 13,543,462 / 13,543,073 | |
| `internal_commuters` | 13,335,103 | 98.5% of resident workers; the rest work out of state (RAC includes them, `od_main` does not) |
| `commute_out` / `commute_in` | 0 / 0 | Expected: nothing in Texas is outside the area |
| `flows` | 6,596,749 block-group pairs | |
| `road_nodes` / `road_edges` / `road_km` | 1,969,256 / 2,599,358 / 747,040 km | Clip box covers the whole state |
| `scenario_road_nodes` / `scenario_road_links` | 1,092,465 / 1,712,605 | Simplification removes 45% of nodes, as on Travis |
| `transit_routes` | 691 | 65 rail routes (33 light rail, 22 train, 6 tram, 4 monorail): DART light rail and Silver Line, Houston METRORail, CapMetro Red Line (route 550), TEXRail, TRE, DCTA A-Train, three Amtrak services |

Not yet done: loading the statewide scenario into sim-core. With 18.6k zones and 1.1M road nodes it is far beyond the Travis timing; the statewide performance chain (F31–F33) addresses that with coarser zones and a binary format.

## First ACS and TIGER build (2026-11-27)

Actions run [45946718823](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45946718823), `--area 48453`, year 2023, `skip_osm: true`. Geofabrik refused connections from Actions at the time, so this run checked the census inputs only and did not publish a release.

| Input or manifest field | Value | Check |
|---|---|---|
| ACS B19013 and B25044 summary files, Texas rows | 18,638 each | Exact match to the Texas block group count, so the `1500000US48` filter in `fetch.sh` keeps every block group |
| TIGER `tl_2020_48_bg.dbf` | read | Source line "US Census Bureau, TIGER/Line 2020 block groups (ALAND)" in the manifest |
| `acs_income_zones` | 703 of 766 | 63 block groups have suppressed or missing median income and take the household-weighted tract value, or the county value if the whole tract is suppressed |
| `acs_zero_car_zones` | 761 of 766 | 5 fall back the same way |
| `area_zones` | 766 of 766 | Every zone gets ALAND |

Still to do: a full build with roads once Geofabrik answers again, which republishes `data-48453-2023` with these fields.

## Zone levels and commute shares (2026-11-27)

- Actions run [45947685279](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45947685279), `--area 48 --level tract`, census only: 6,896 zones, the exact 2020 Texas tract count; population 29,145,505 and internal commuters 13,335,103, the same as the block-group build; 3,878,003 tract-to-tract flows (6,596,749 at block group); build 5.5 s without roads. 18,625 of 18,638 block groups have a TIGER land area.
- Actions run [45948912088](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45948912088), Travis, census only: `commute_mode_shares` sums 766 block groups to 736,639 workers, exactly the published Travis County `B08301_E001`. Shares: drove alone 0.5973, carpool 0.0746, transit 0.0167, walk 0.0207, bike 0.0060, work from home 0.2718, other 0.0129.

## GTFS headways (2026-11-27)

Actions run [45949840370](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45949840370), Travis with roads, branch build (not published). The CapMetro feed had 26,147 trips; 143 of 158 OSM routes matched a GTFS `route_short_name` (`gtfs_routes_matched`). Sample, peak / midday headway in minutes:

| Route | Peak | Midday | Span (h) |
|---|---|---|---|
| 801 MetroRapid | 10.9 | 9.4 | 18.6 |
| 803 MetroRapid | 10.0 | 10.3 | 18.5 |
| 7 | 17.1 | 16.7 | 19.5 |
| 10 | 15.0 | 15.8 | 19.1 |
| 20 | 17.1 | 14.3 | 19.5 |
| 1 | 30.0 | 30.0 | 18.7 |
| 550 Red Line | 40.0 | 60.0 | 12.8 |

The Red Line reads 40 / 60 min in the current feed (19 trips per direction), sparser than the 32 / 51 min in `figures.csv`, which came from the published timetable earlier. A count-based mean is coarse for a line with three trains in the peak window, so the timetable figures stay the calibration reference for route 550. That run took the busier direction for every window, which gave the peak-only express 985 no peak headway; windows now take the more frequent direction.


## Existing lines (2026-11-27)

Actions run [45951673767](https://github.com/mr-gril-em-up/Transportation-Simulator/actions/runs/45951673767), Travis with roads, branch build: `scenario.json` carries 61 existing lines (`existing_lines`), one per GTFS-matched route number, and passes sim-core `validate()`. The same day's main build (run 45951703196) republished `data-48453-2023` with GTFS headways (143 routes) and `commute_mode_shares`, but without lines.
