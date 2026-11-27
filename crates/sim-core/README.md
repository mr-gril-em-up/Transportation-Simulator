# sim-core

Aggregate zone-to-zone travel model for Transpo-Sim: trip generation, desired
arrival times, mode choice, and road/transit assignment. Headless Rust library;
the app (Tauri) and the data pipeline depend on it.

## Input schema (contract with `data-pipeline`)

One JSON file per region, deserialized into `ScenarioInput`
([src/schema.rs](src/schema.rs) is the source of truth). Coordinates are WGS84
degrees, money is 2024 USD, ids are `u32` (unique per list, need not be dense).
Fields marked optional may be omitted.

```jsonc
{
  "zones": [{
    "id": 1, "lon": -97.75, "lat": 30.27,   // centroid
    "population": 2000,                      // residents
    "jobs": 6000,                            // LODES WAC jobs in zone
    "road_node": 10,                         // nearest drivable road node id
    "zero_car_share": 0.06,                  // optional, ACS B08201
    "median_income": 70000,                  // optional, ACS B19013
    "parking_cost": 0,                       // optional, $/day at destination
    "area_km2": 2.1                          // optional, else estimated
  }],
  "road_nodes": [{ "id": 10, "lon": -97.75, "lat": 30.27 }],
  "road_links": [{
    "from": 10, "to": 20, "length_m": 6200, "speed_kph": 60,
    "lanes": 1,            // optional, per direction
    "capacity_vph": 1800,  // optional, per direction; derived from speed x lanes
    "toll": 0,             // optional, $ per traversal
    "oneway": false        // optional; false = usable both ways
  }],
  "commutes": [{ "home": 2, "work": 1, "workers": 900 }],  // LODES OD by zone
  "lines": []              // optional, transit present at start (see TransitLine)
}
```

`TransitLine`: `{ "name", "mode", "stops": [{ "lon", "lat", "name"?, "park_ride"? }],
"headway_min", "fare"?, "fare_per_km"?, "speed_kph"?, "span_hours"? }`.
Modes: `bus, trolleybus, brt, tram, light_rail, commuter_rail, metro,
high_speed_rail, monorail, people_mover, gondola, maglev, ferry` (no hyperloop).

`ScenarioInput::validate()` checks ids and ranges; the pipeline should run it on
its output. A small working example is
[tests/fixtures/tiny.json](tests/fixtures/tiny.json).

M0/M1 zones are Travis County block groups. Keep the road graph simplified
(degree-2 nodes contracted); tens of thousands of nodes is fine.

## Public API

Lands in the next PR (`Sim`: build from a scenario, add/update/remove lines with
sub-2-second re-evaluation, read results, advance the shared clock).
