# Architecture

How Transpo-Sim is put together: the components, the data that moves between them, and the rules every component must keep. The design rationale lives in the [design doc](https://claude.ai/artifact/Hjw86789MWbzev8BMVZBz8); this page is the engineering view of it, plus what has been built so far. Decisions are logged in [DECISIONS.md](DECISIONS.md).

Last updated 2026-11-27.

## One-paragraph summary

A Rust data pipeline turns public data (Census, LODES, OpenStreetMap) into a game package once per release. A headless Rust library, `sim-core`, loads that package and runs the travel model: aggregate zone-to-zone flows for the whole state, with individual agents only near the camera. A Tauri desktop app links `sim-core` as native code, talks to it through a message interface, and draws the map in its webview with MapLibre GL (deck.gl planned for the local tier in M2, see [app decision 2](../app/docs/DECISIONS.md)). There is no server.

As built today (2026-11-27): the pipeline writes `scenario.json` for Travis County; sim-core has the input schema but no `Sim` API yet; the app runs against a TypeScript mock sim behind the same client interface it will use for sim-core.

## Components

```mermaid
flowchart LR
    subgraph offline["Offline, once per release"]
        raw["Public data<br/>Census, LODES, ACS, PUMS,<br/>OSM, TIGER, DEM, NLCD, GTFS"]
        pipe["data-pipeline<br/>crates/data-pipeline<br/>(tsim-data binary)"]
        raw --> pipe
    end
    pkg[("Game package<br/>zones, flows, roads,<br/>transit, population")]
    pipe --> pkg
    subgraph app["Desktop app (Tauri)"]
        core["sim-core<br/>crates/sim-core<br/>headless Rust library"]
        shell["Tauri shell (Rust)<br/>app/"]
        web["Webview UI (TypeScript)<br/>MapLibre GL<br/>(deck.gl planned, M2)"]
        core <-->|"Rust API"| shell
        shell <-->|"commands / events"| web
    end
    pkg --> core
```

| Component | Path | Language | Responsibility | Docs |
|---|---|---|---|---|
| Cargo workspace | `Cargo.toml` | Rust | Builds every crate under `crates/*` together | this page |
| data-pipeline | `crates/data-pipeline` | Rust | Download, clip and join public data; write the game package | [README](../crates/data-pipeline/README.md) |
| sim-core | `crates/sim-core` | Rust | Demand model, mode choice, assignment, clock, economy; owns the input schema | [README](../crates/sim-core/README.md) |
| app | `app/` | Rust (Tauri) + TypeScript | Desktop shell, map rendering, player tools; no simulation logic. `app/src-tauri` is its own Cargo workspace, outside `crates/*` | [README](../app/README.md), [app docs](../app/docs/ARCHITECTURE.md) |

### Dependency direction

`app` will depend on `sim-core` as a path dependency (stubbed, commented out in `app/src-tauri/Cargo.toml` until the `Sim` API exists). `data-pipeline` depends on `sim-core` only for the input schema types. `sim-core` depends on neither. Keep it that way: the design doc requires the core to stay a headless library so a future client swap does not touch it, and keeps the zone hierarchy and population decoupled from transport code so the city-builder path stays open.

## Data flow

1. **Build time.** `tsim-data` reads raw files from `crates/data-pipeline/data/raw` (fetched by `scripts/fetch.sh`, gitignored) and writes the package to `--out`. Travis County (`48453`) for M0/M1; all of Texas (`--area 48`) from M3.
2. **Load.** `sim-core` deserializes the package into `ScenarioInput` and runs `validate()`.
3. **Run.** `sim-core` computes skims, demand, mode split and assignment. A player edit (add or change a line) triggers incremental re-evaluation, target under 2 s per metro.
4. **Display.** The app requests results through the message interface and renders layers: desire lines, link volumes, line ridership, agents near the camera, and the tube schematic at State and Region zoom.

## Simulation tiers

Three tiers run on one clock. Only the camera's neighborhood ever becomes individual agents.

| Tier | Scope | Zone unit | Method | Update rate | Milestone |
|---|---|---|---|---|---|
| Strategic | All of Texas, always | Tract (~6,900) | OD matrices, logit mode split, static assignment on contracted highway graph | Once per game day, incremental | M3 |
| Regional | Metros built in or viewed | Block group | Mesoscopic queues, spillback, per-trip transit loads | Every 5–15 game min | M1 (Travis), M3 (all metros) |
| Local | ~5 km around camera | Block, then people | Agents sampled from regional flows | Real time | M2 |

### Invariants (enforce in code and tests)

- **Flows go down, corrections go up.** Local agents are sampled from regional flows; regional flows are disaggregated from strategic OD; measured travel times feed back up as link-cost corrections.
- **The camera never changes the score.** Budget, ridership and approval are computed from the strategic and regional tiers only.
- **Determinism.** Sampling is seeded per zone and time slice. Same seed, same people on the same bus after zooming out and back in. Saves hold aggregate state plus seeds, never agents.
- **One clock.** Zoom changes which tiers are drawn, never how fast time runs. Time-lapse speeds every tier together. Above 10x the local tier draws flow animation instead of agents.

## Interfaces

### Pipeline to sim-core: `ScenarioInput`

Owned by sim-core. Source of truth: [`crates/sim-core/src/schema.rs`](../crates/sim-core/src/schema.rs), documented in the [sim-core README](../crates/sim-core/README.md#input-schema-contract-with-data-pipeline). One JSON file per region: `zones`, `road_nodes`, `road_links`, `commutes`, optional `lines`. Coordinates WGS84 degrees, money 2024 USD, ids `u32`. A change to this schema is a breaking change for the pipeline and needs both READMEs updated in the same PR.

### sim-core to app: `Sim` API and Tauri messages

sim-core's side is not built yet (roadmap F15). Planned surface from the sim-core README: build a `Sim` from a scenario, add/update/remove lines with sub-2-second re-evaluation, read results, advance the shared clock.

The app's side exists: [`app/src/sim/api.ts`](../app/src/sim/api.ts) defines `SimClient` (`upsertRoute`, `removeRoute`, each returning results for every route) with `RouteSpec` modes copied from sim-core's `TransitLine`. Today `createSim()` returns a TypeScript `MockSim`. The swap plan (Tauri state holding `sim_core::Sim`, `upsert_route`/`remove_route` commands, a `TauriSim` client) is in [app ARCHITECTURE, "Swapping in sim-core"](../app/docs/ARCHITECTURE.md#swapping-in-sim-core). The webview never holds simulation state. The game clock lives in the client until sim-core exposes clock advance (app decision 9).

### Pipeline output files

The pipeline writes `scenario.json` as `sim_core::ScenarioInput` directly (it depends on sim-core and calls `validate()` before writing), plus detail files sim-core does not need, keyed by the same ids: `zones.json` (GEOID, worker and commute counts), `roads.json` (OSM class, name, drawing shape, **total** lanes), `transit.json` (existing OSM routes, for calibration and the keep-existing-transit toggle) and `manifest.json` (summary, top 20 flows). Full spec: [data-pipeline OUTPUT_FORMAT.md](../crates/data-pipeline/docs/OUTPUT_FORMAT.md).

Traps for consumers:

- `road_links[].lanes` in `scenario.json` is **per direction** (two-way OSM total halved, minimum 1). `roads.json` `edges[].lanes` is the **total**. Do not mix them.
- Zone and road-node ids equal array positions in the detail files, but are not stable across areas or OSM snapshots. Use GEOID or OSM id for anything that must survive a rebuild.
- `zero_car_share`, `median_income`, `parking_cost`, `area_km2` are written as sim-core defaults until ACS and TIGER inputs are added (roadmap F31).
- OSM transit `mode` values (`train`, `subway`, ...) are not sim-core `Mode` values; mapping (for example `train` to `commuter_rail` for the Red Line) is the consumer's job (F19, F44).

## Repository layout

```
Cargo.toml              Rust workspace root (members = crates/*)
crates/
  sim-core/             simulation library, input schema
  data-pipeline/        tsim-data binary, public data -> game package
app/                    Tauri desktop app: TypeScript client (src/), Rust shell (src-tauri/, own workspace)
docs/                   this hub: roadmap, architecture, decisions, docs standard
.github/workflows/      CI (data-pipeline workflow can build a package in Actions)
```

## As-built deviations from the design doc

| Design doc says | Built | Notes |
|---|---|---|
| Data pipeline in Python (osmnx, geopandas, pandas) | Rust crate `data-pipeline` | One toolchain, compile-time schema check, statewide PBF speed, few dependencies. See [pipeline architecture](../crates/data-pipeline/docs/ARCHITECTURE.md) and D14 |
| Census tract and block-group polygons from TIGER | Block-group population-weighted centroids only | Polygons deferred until the map needs zone fills |

## Open engineering questions

- **Licence.** All crates inherit the workspace `license = "UNLICENSED"`. Decide before any release.
- **Package format.** JSON is interim. A binary format and tile streaming land with M3 (F33).
- **OSM ODbL.** The derived road database ships with the game; it needs attribution and share-alike review before any sale (F46).
