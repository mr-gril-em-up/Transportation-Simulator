# Transpo-Sim

A transportation-network game on the real map of Texas. Around 31 million synthetic residents, each with a real home block, job block and daily schedule, built from public Census and OpenStreetMap data. The player is the state transportation authority: build roads, transit and pricing that move people faster and cheaper than the car-only baseline without going broke. Zoom runs from statewide desire lines down to a single bus stop, and the numbers stay consistent the whole way.

Single player desktop game. Rust simulation core, Tauri app shell, map drawn with MapLibre GL and deck.gl. No server.

## Status

Pre-alpha, milestone **M0** (data spike on Travis County). Target: Texas MVP as v0.5.0, estimated 2027-03-26. See the [roadmap](docs/ROADMAP.md).

| Milestone | Est. date | State |
|---|---|---|
| M0 Data spike on Travis County (v0.1.0) | 2026-12-08 | In progress |
| M1 The loop on Travis County (v0.2.0) | 2027-01-05 | Planned |
| M2 Money, clock and agents (v0.3.0) | 2027-01-29 | Planned |
| M3 Statewide (v0.4.0) | 2027-03-05 | Planned |
| M4 Schematic and polish, Texas MVP (v0.5.0) | 2027-03-26 | Planned |

## Repository layout

| Path | What it is | Docs |
|---|---|---|
| `crates/sim-core` | Headless Rust library: demand, mode choice, assignment, clock, economy. Owns the input schema | [README](crates/sim-core/README.md) |
| `crates/data-pipeline` | `tsim-data` binary: public data to game package | [README](crates/data-pipeline/README.md) |
| `app/` | Tauri desktop app and map UI | [README](app/README.md) |
| `docs/` | Roadmap, architecture, decisions, docs standard | [index](docs/README.md) |

## Build and test

Requires a stable Rust toolchain (`rustup`). From the repo root:

```sh
cargo build --workspace
cargo test --workspace
```

Component-specific steps (data downloads, running the pipeline, launching the app) are in each component's README under **Run** and **Test**.

## Documentation

Docs are part of every change. Read the [docs standard](docs/DOCS_STANDARD.md) before opening a PR; the [PR template](.github/pull_request_template.md) carries its checklist.

- [Docs index](docs/README.md): every doc in the repo
- [Architecture](docs/ARCHITECTURE.md): components, data flow, simulation tiers, interfaces
- [Roadmap](docs/ROADMAP.md): features, milestones, Gantt chart
- [Decision log](docs/DECISIONS.md): what was decided and why
- [Design doc](https://claude.ai/artifact/Hjw86789MWbzev8BMVZBz8): the approved game design

## Data and licences

Built from OpenStreetMap (ODbL: attribution required, derived database stays open), Census LODES, ACS, PUMS and TIGER/Line. "Map data © OpenStreetMap contributors." Crates are `UNLICENSED` (workspace setting) until a licence is chosen.
