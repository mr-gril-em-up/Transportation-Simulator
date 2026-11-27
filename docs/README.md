# Documentation index

Start here. Every doc in the repo is listed below. New docs get a row in the same PR that adds them ([docs standard](DOCS_STANDARD.md#5-roadmap-and-architecture-upkeep)).

## Project

| Doc | What it answers | Owner |
|---|---|---|
| [Design doc](https://claude.ai/artifact/Hjw86789MWbzev8BMVZBz8) (external) | What the game is, the player loop, demand model, economics, modes, simulation tiers, MVP scope. Approved by Collins | Collins |
| [ROADMAP.md](ROADMAP.md) | Every feature through the Texas MVP, milestones M0–M4, dependencies, exit criteria, estimated dates (Gantt) | docs |
| [DECISIONS.md](DECISIONS.md) | What was decided, by whom, and why | everyone |

## Engineering

| Doc | What it answers | Owner |
|---|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Components, data flow, simulation tiers and their invariants, interfaces and their traps for consumers | docs |
| [DOCS_STANDARD.md](DOCS_STANDARD.md) | What every component's docs must cover; PR checklist | docs |

## Components

| Component | Doc | What it covers |
|---|---|---|
| sim-core | [crates/sim-core/README.md](../crates/sim-core/README.md) | Travel model library; `ScenarioInput` input schema (contract with the pipeline); `Sim` API. Deeper docs: [MODEL](../crates/sim-core/docs/MODEL.md), [ARCHITECTURE](../crates/sim-core/docs/ARCHITECTURE.md) |
| data-pipeline | [crates/data-pipeline/README.md](../crates/data-pipeline/README.md) | `tsim-data`: Census, LODES and OSM to `scenario.json` plus detail files. Deeper docs: [ARCHITECTURE](../crates/data-pipeline/docs/ARCHITECTURE.md), [OUTPUT_FORMAT](../crates/data-pipeline/docs/OUTPUT_FORMAT.md), [DATA_SOURCES](../crates/data-pipeline/docs/DATA_SOURCES.md), [ASSUMPTIONS](../crates/data-pipeline/docs/ASSUMPTIONS.md), [TESTING](../crates/data-pipeline/docs/TESTING.md) |
| app | [app/README.md](../app/README.md) | Tauri shell and MapLibre map client, line drawing, tube schematic, clock; mock sim until sim-core's API lands. Deeper docs: [ARCHITECTURE](../app/docs/ARCHITECTURE.md), [DECISIONS](../app/docs/DECISIONS.md), [MOCK-SIM](../app/docs/MOCK-SIM.md), [SCHEMATIC](../app/docs/SCHEMATIC.md), [ROADMAP-APP](../app/docs/ROADMAP-APP.md) |

## For agent developers

[CLAUDE.md](../CLAUDE.md) at the repo root points agents at these docs and the rules they must follow.
