# Assumptions and defaults

Every number or rule in the code that is not read straight from a source. Each entry says where it lives, why it was chosen, and what replaces it.

## Zones and demand

| Assumption | Where | Why | Replace with |
|---|---|---|---|
| Zones are 2020 block groups | `census.rs` | Design doc's unit for City zoom and the M0/M1 model; ~18,600 statewide is tractable | Tract zones for the statewide strategic tier come from the same data by aggregating on the first 11 GEOID digits |
| All jobs (`JT00`, `S000`/`C000`) | `census.rs`, `fetch.sh` | Total commute demand; wage and sector splits are not used by the model yet | Read `SE01–03` / `SI01–03` when mode choice or destination choice needs them |
| Population is 2020 census, jobs are LODES 2023 | `lib.rs` | Latest of each; three years apart | ACS 5-year population if growth since 2020 matters for calibration |
| Out-of-state commuters ignored | `census.rs` | `od_aux` is a rounding error for Travis | Read `tx_od_aux_*` for border metros (El Paso, Texarkana) at M3 |
| `median_income` = ACS B19013; `zero_car_share` = ACS B25044 no-vehicle households / households; ACS 5-year ending in the LODES year | `acs.rs` | Block-group level; B25044 counts the same households as B08201 (named in sim-core's schema), which is tract-only (DATA-4) | Tract-level B08201 cross-check |
| Tract and county zones use population-weighted means of block-group centroid, income and zero-car share | `aggregate.rs` | Households per block group are not kept on the zone; population tracks households closely and is exact | Carry households to weight income and zero-car share exactly |
| Suppressed or zero-household ACS block groups take the household-weighted tract mean, then county mean; sim-core default (0.06, 70,000) only if both are empty | `acs.rs` `apply` | Neighbours in the same tract are the best available estimate; a statewide default would flatten rich and poor areas | Use ACS margins of error to blend small estimates |
| `parking_cost` 0 | `scenario.rs` | No source yet | Downtown parking survey or meter rates |
| `area_km2` = TIGER 2020 `ALAND` (land only, water excluded) | `acs.rs` | Density should count land people can live on | n/a |
| `area_km2` omitted | `scenario.rs` | sim-core estimates it from zone spacing | TIGER `ALAND` (M1) |

## Road network

| Assumption | Where | Why | Replace with |
|---|---|---|---|
| Classes kept: motorway, trunk, primary, secondary, tertiary, their links, unclassified, residential, living_street | `osm.rs` `class_defaults` | Covers every road a commute trip uses; dropping `service` (parking aisles, driveways) roughly halves the graph. Residential and unclassified stay for M1 (DATA-3): line edits already meet the 2 s target, so cutting them would trade accuracy for model-load time only | Add `service` if local-tier agents need driveways; consider a class cut for the statewide tiers (F34) |
| Default speeds (km/h): motorway 105, trunk 90, primary 72, secondary 64, tertiary 56, motorway/trunk links 72, other links 48, unclassified 48, residential 40, living_street 16 | `osm.rs` | Typical Texas posted limits (65, 55, 45, 40, 35, 30, 25 mph) where `maxspeed` is untagged | TxDOT speed data or calibration against observed travel times |
| Default total lanes: motorway 4, trunk 4, primary 4, secondary 2, tertiary 2, links 1, unclassified 2, residential 2, living_street 1; one-way roads get half (rounded up) | `osm.rs` | Common US cross-sections | Tagged `lanes` (already used when present) |
| `maxspeed` without a unit is km/h; `"NN mph"` is converted; values that are zero, negative, non-numeric or above 200 km/h fall back to the class default | `osm.rs` `parse_speed` | OSM convention. The first statewide build failed validation on a link tagged with a zero or unparseable speed that the Travis build never met | n/a |
| Motorways and roundabouts are one-way unless `oneway=no` | `osm.rs` | OSM convention: motorway carriageways are mapped separately per direction | n/a |
| Lanes per direction in the scenario = total / 2 for two-way roads (min 1) | `scenario.rs` | sim-core's `lanes` is per direction | n/a. Odd totals (3 = two lanes plus a center turn lane) round down on purpose |
| Clip box = zone bbox + 0.05° | `lib.rs`, `--margin` | Keeps the roads just past the county line that boundary zones use | Clip by county polygon if the bbox pulls in too much of neighboring counties |
| Edges that cross the clip box are dropped, not cut | `osm.rs` `build_roads` | No dangling half-edges; the margin makes the loss irrelevant | n/a |
| Zone connector = nearest node on a non-motorway, non-trunk road in the largest connected component, straight line | `scenario.rs` | One connector per zone is what sim-core's schema takes; freeway nodes would make access free; fragments cut off by clipping would leave a zone with no paths | Multiple connectors per zone if sim-core adds them |
| Routing graph in scenario.json is simplified: fragments dropped, dead ends without a zone pruned, identical pass-through links merged | `scenario.rs` | None of these can carry a zone-to-zone trip or change a path length, and sim-core's run time scales with nodes (DATA-2) | Keep a node if sim-core later loads transit stops or non-work destinations onto road nodes: add it to the protected set |

## Transit

| Assumption | Where | Why | Replace with |
|---|---|---|---|
| Stops = relation members with role `stop*`, else `platform*` | `osm.rs` | PTv2 tagging; older routes map only platforms | GTFS for agencies that publish it |
| Routes kept if any stop is in the clip box | `osm.rs` | Lines entering the county count | n/a |
| No headways or schedules | `osm.rs` | OSM does not carry them | Existing lines come from agency GTFS instead (DATA-9) |
| A count station belongs to the nearest link within 100 m whose OSM `ref` or `name` matches its `ON_ROAD` | `counts.rs` | Route numbers are shared between TxDOT and OSM; distance alone picks frontage roads | Linear referencing on TxDOT's roadway inventory |
| Station ids ending `[NSEW]BSR` are one-direction frontage-road counts and `[NSEW]BDR` one-direction direct-connector counts | `counts.rs` | TxDOT suffix convention; their AADT (a few thousand) next to freeway main lanes (150k) only fits a frontage road | TxDOT roadway inventory linkage |
| A frontage count never sits on `motorway`, `motorway_link` or `trunk_link`; `trunk` is allowed | `counts.rs` | Texas freeways are mostly `motorway` in OSM; `trunk` is often an at-grade divided highway that is the only road there | Exclude `trunk` where OSM tags a freeway as trunk |
| A station is external when its `CNTY_NM` county is outside `--area` | `counts.rs` | TxDOT's own county attribution; no zone polygons yet to test against | Point-in-polygon once zone polygons exist (F31) |
| AADT is the station's latest annual figure, whatever its year | `counts.rs` | TxDOT counts on a cycle; the latest year is the best available | Filter or factor by year |
| A route's stops are its longest trip that day; short-turn and branch patterns are dropped | `gtfs.rs` | sim-core lines have one stop list; the longest trip covers the most stations | One line per pattern if branch service matters |
| A route with no departures 07–09 or 10–15 on the chosen Wednesday is not an existing line (PM-only expresses, school trippers: 31 of 446 statewide routes) | `lines.rs` | sim-core lines need a base headway | Add a PM peak window |
| Headways are means over 07–09 and 10–15 on the busiest Wednesday, both directions combined per window | `gtfs.rs` | Frequency-based model, DATA-6 | Per-direction headways |
| Scenario `lines` empty | `scenario.rs` | Design doc: game starts with existing transit removed | Optional toggle in the app reads `transit.json` |

## Known gaps

1. No zone polygons yet (map draws points or desire lines).
2. ACS fields are block-group estimates with large margins of error; suppressed ones use tract or county means.
3. No synthetic population or non-work trips (later milestones).
4. JSON output will be large statewide; binary format pending a sim-core loader.
5. Raw file names hard-code Texas (`48`, `tx`, `texas-latest`).
