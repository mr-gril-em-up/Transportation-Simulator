# Output format

A build writes five files to `--out`. All are JSON. `scenario.json` follows sim-core's schema exactly (`crates/sim-core/src/schema.rs`); the other files carry detail sim-core does not need, keyed by the same ids.

## Id rules

- **Zone id** = position in `zones.json` `zones` array = `id` in `scenario.json` `zones`. Zones are sorted by GEOID, so ids are stable for the same inputs and area.
- **Road node id** = position in `roads.json` `nodes` = `id` in `scenario.json` `road_nodes`. `scenario.json` holds only the nodes left after routing simplification (see below), so its ids are a sparse subset; look coordinates up by id, not position.
- Ids are dense (0..n-1) but consumers should not rely on that; sim-core's schema allows sparse ids.
- Ids are not stable across different areas or OSM snapshots. Use GEOID (zones) or OSM id (transit) for anything that must survive a rebuild.

## Units and conventions

Coordinates are WGS84 longitude, latitude in degrees. Lengths in meters, speeds in km/h, counts are people or jobs.

## `scenario.json` (sim-core `ScenarioInput`)

```json
{
  "zones": [{"id": 0, "lon": -97.72, "lat": 30.30, "population": 2100.0, "jobs": 0.0,
             "road_node": 12, "zero_car_share": 0.06, "median_income": 70000.0,
             "parking_cost": 0.0, "area_km2": null}],
  "road_nodes": [{"id": 0, "lon": -97.745, "lat": 30.265}],
  "road_links": [{"from": 0, "to": 1, "length_m": 590.2, "speed_kph": 56.3, "lanes": 2,
                  "capacity_vph": null, "toll": 0.0, "oneway": false}],
  "commutes": [{"home": 0, "work": 1, "workers": 500.0}],
  "lines": []
}
```

| Field | Filled from | Notes |
|---|---|---|
| `zones[].population` | Centers of Population `POPULATION` | 2020 census |
| `zones[].jobs` | LODES WAC `C000` summed to block group | |
| `zones[].road_node` | Nearest node on a non-motorway, non-trunk road in the largest connected component | See ARCHITECTURE.md step 6 |
| `zones[].median_income` | ACS 5-year B19013 | Tract, then county household-weighted fallback where suppressed; 70000 if no ACS file |
| `zones[].zero_car_share` | ACS 5-year B25044 (no-vehicle households / households) | Same fallback; 0.06 if no ACS file |
| `zones[].area_km2` | TIGER/Line 2020 `ALAND` / 10⁶ | `null` if no TIGER file |
| `zones[].parking_cost` | Not sourced | 0 |
| `road_links[].lanes` | OSM `lanes`, else class default | Converted to **per direction**: one-way keeps the total, two-way halves it (minimum 1) |
| `road_links[].class` | OSM `highway` value (`motorway`, `motorway_link`, `trunk`, `primary`, ...) | sim-core sets capacity per facility type from it (SIM-23); contraction never merges links of different class |
| `road_links[].capacity_vph`, `toll` | Not set | sim-core derives capacity from class, speed and lanes |
| `road_links[].oneway` | OSM `oneway`, motorway class, roundabouts | `oneway=-1` ways are flipped so `from → to` is the legal direction |
| `commutes[]` | LODES OD `S000` summed to block-group pairs | Only pairs with both ends in the area |
| `lines` | Empty by default. With `--existing-lines`, today's network: one line per GTFS route of every agency feed (DATA-7, DATA-9) | sim-core loads these as existing service (no capital charge, open from day 0), so they are opt-in until the app has a setting to drop them and Collins decides the default. The same lines are always written to `existing_lines.json`. Red Line 550 uses the `figures.csv` timetable headways; Leander, Lakeline and Howard stops carry `park_ride` and `park_ride_spaces` (SIM-11) |

Zero-length links and self-loops are dropped from `scenario.json` (they remain in `roads.json`).

`scenario.json`'s road graph is simplified for routing (DATA-2): fragments outside the largest connected component are dropped, dead ends without a zone are pruned, and chains of identical links through non-connector nodes are merged into one link (`length_m` summed). A scenario link therefore may span several `roads.json` edges; to draw it, follow `roads.json` from `from` to `to`. Zone-to-zone paths and lengths are unchanged.

## `zones.json`

```json
{"zones": [{"geoid": "484530011001", "lon": -97.745, "lat": 30.27, "population": 1500,
            "resident_workers": 500, "jobs": 5000, "commute_out": 30, "commute_in": 600,
            "median_income": 90000.0, "zero_car_share": 0.1, "area_km2": 0.25}]}
```

| Field | Meaning |
|---|---|
| `geoid` | 2020 GEOID of the zone: 12 digits for a block group, 11 for a tract, 5 for a county (`--level`) |
| `lon`, `lat` | Population-weighted centroid |
| `population` | 2020 census population |
| `resident_workers` | LODES RAC `C000`: jobs held by people living here (anywhere in the US) |
| `jobs` | LODES WAC `C000`: jobs located here |
| `commute_out` | Workers living here, working in Texas outside the area |
| `commute_in` | Jobs here held by Texas residents living outside the area |
| `median_income`, `zero_car_share`, `area_km2` | As in `scenario.json` (after fallback); omitted when the ACS or TIGER file was absent |

`resident_workers` is usually larger than (internal flows from the zone + `commute_out`) because RAC includes out-of-state work locations that `od_main` lacks.

## `roads.json`

```json
{"nodes": [[-97.745, 30.265], [-97.743, 30.27]],
 "edges": [{"from": 0, "to": 1, "length_m": 590.2, "class": "primary", "speed_kph": 56.3,
            "lanes": 4, "oneway": false, "name": "Congress Avenue",
            "shape": [[-97.744, 30.267]]}]}
```

| Field | Meaning |
|---|---|
| `nodes[i]` | `[lon, lat]` of road node `i` (an intersection or dead end) |
| `edges[].class` | OSM `highway` value |
| `edges[].lanes` | **Total** lanes across both directions (one-way: lanes in that direction). Differs from `scenario.json`, which is per direction |
| `edges[].name` | OSM `name`, omitted if untagged |
| `edges[].ref` | OSM `ref` (route numbers, e.g. `I 35;US 290`), omitted if untagged |
| `edges[].shape` | Intermediate points between `from` and `to`, in travel order; omitted if the edge is straight |

## `existing_lines.json`

A JSON array of sim-core `TransitLine`s: today's network, built by `lines::existing_lines` (DATA-7, DATA-9) from every agency GTFS feed read, whenever roads are built. Names start with the agency (`CapMetro`, `DART`, `METRO`, `Trinity Metro`, `DCTA`, `VIA`, `CCRTA`, `El Paso County Transit`, `Sun Metro`), then the route's short and long name. sim-core's calibration reads it, and a future app setting "start with today's network" can too; `scenario.json` `lines` holds the same array only with `--existing-lines`.

## `transit.json`

```json
{"routes": [{"osm_id": 200, "mode": "bus", "ref": "801", "name": "MetroRapid 801",
             "operator": "CapMetro",
             "stops": [{"osm_id": 20, "name": "Congress/5th", "lon": -97.7431, "lat": 30.2701}]}]}
```

`mode` is the OSM `route` value: `bus`, `trolleybus`, `tram`, `light_rail`, `train`, `subway`, `monorail`. In OSM each direction of a line is usually its own relation, so expect two routes per line. Routes whose `ref` matches a CapMetro GTFS `route_short_name` also carry `peak_headway_min` (07:00–09:00), `midday_headway_min` (10:00–15:00) and `span_hours`, for the busiest Wednesday in the feed (see ARCHITECTURE step 5b, DATA-6). A field is omitted when the route has no departures in that window or no GTFS match. Mapping to sim-core's `Mode` (for example `train` → `commuter_rail` for CapMetro's Red Line) is left to the consumer because it depends on the service, not the tag.

## `counts.json`

TxDOT traffic count stations snapped to scenario links (DATA-10), written when `raw/txdot_aadt.csv` was downloaded and roads were built. A JSON array:

```json
[{"station_id": "227_7101", "year": 2023, "aadt": 187000, "on_road": "IH0035",
  "lon": -97.7321, "lat": 30.2803, "from": 1200, "to": 1201, "link": 40211,
  "from2": 1350, "to2": 1349, "link2": 40388, "active": true,
  "facility": "main", "external": false, "dist_m": 12.4},
 {"station_id": "227H136SBSR", "year": 2025, "aadt": 3148, "on_road": "SL0001",
  "lon": -97.79, "lat": 30.33, "from": 880, "to": 881, "link": 51002,
  "active": true, "facility": "frontage", "direction": "S", "external": false,
  "dist_m": 8.0}]
```

| Field | Meaning |
|---|---|
| `station_id` | TxDOT `TRFC_STATN_ID` |
| `year`, `aadt` | Latest report year and annual average daily traffic, both directions combined |
| `on_road` | TxDOT `ON_ROAD` (`IH0035`, `FM2222`, street names) |
| `lon`, `lat` | Station location |
| `from`, `to` | Scenario road link (node ids in `scenario.json`) the station snaps to |
| `link`, `link2` | Index of `from`-`to` (and of `from2`-`to2`) in `scenario.json` `road_links`; tells parallel links between the same two nodes apart |
| `active` | TxDOT `ACTIVE`: the station is still counted (inactive stations keep their last AADT) |
| `from2`, `to2` | Opposite carriageway when `from`-`to` is one-way and one was found; compare `aadt` with the sum of both links. Omitted otherwise |
| `facility` | From the id suffix: `main` (main lanes, two-way AADT), `frontage` (`..NBSR` etc., one direction of a frontage road) or `ramp` (`..NBDR` etc., one direction of a direct connector). Frontage and ramp counts never get `from2`/`to2` |
| `direction` | `N`, `S`, `E` or `W` for `frontage` and `ramp`; omitted for `main` |
| `external` | The station's county (TxDOT `CNTY_NM`) is outside `--area`. County builds keep roads a margin past the line, where there are no zones; skip these when comparing against assigned volumes |
| `dist_m` | Station to `from`-`to` distance, meters (at most 100) |

TxDOT's layer has no K factor, D factor or truck share, so none are written. Releases also carry it as a separate asset, `counts-<area>-<year><suffix>.json`.

## `manifest.json`

Human-readable summary, pretty-printed. Fields: `area`, `lodes_year`, `sources`, `zones`, `population`, `resident_workers`, `jobs`, `flows`, `internal_commuters`, `commute_out`, `commute_in`, `top_flows` (20 largest flows as GEOID pairs), and when roads were built `bbox`, `road_nodes`, `road_edges`, `road_km`, `transit_routes`, `rail_routes` (every non-bus route as `mode | ref | name | operator | N stops`, plus ` | headway P/M min` when GTFS gave peak/midday headways, sorted, so known lines can be checked from the manifest alone). `acs_income_zones`, `acs_zero_car_zones` and `area_zones` count block groups with their own ACS value or TIGER area (`null` if the file was absent). `commute_mode_shares` (present when B08301 was downloaded) sums the area's block groups into shares of all workers 16+: `drove_alone`, `carpool`, `transit`, `walk`, `bike`, `wfh` and `taxi_motorcycle_other`, which add to 1, plus `workers` and `block_groups`. sim-core's F14 calibration reads it. `existing_lines` counts the lines in `existing_lines.json`. `gtfs_routes_matched` counts OSM routes that got CapMetro GTFS headways. `gtfs_feeds` lists each GTFS feed read with its route count. `count_stations` and `counts_snapped` count TxDOT stations read and written to `counts.json`; `counts_frontage`, `counts_ramp` and `counts_external` count the written entries with that `facility` or `external: true` (all `null` without the download). `road_classes` is `all` or `highway` (`--roads`). `zone_level` is `block_group`, `tract` or `county`; `block_groups` is the number of block groups merged into `zones`. `scenario_road_nodes` and `scenario_road_links` count the simplified graph in `scenario.json`; `road_nodes` and `road_edges` count `roads.json`.

## Distribution

The workflow publishes each build as GitHub Release `data-<area>-<year>` with two assets: `tsim-data-<area>-<year>.tar.gz` (all five files) and `manifest-<area>-<year>.json`. `scripts/download-build.sh` unpacks the archive into `data/out/<area>-<year>/`. Roads and transit in the archive are derived from OpenStreetMap and stay under ODbL with attribution; the release notes say so.

## Changing the format

`scenario.json` changes only when sim-core's schema changes; the compile will fail here first because the pipeline builds sim-core's types directly. The detail files are owned by this crate: update `src/model.rs`, this document, and any reader in `app/`.
