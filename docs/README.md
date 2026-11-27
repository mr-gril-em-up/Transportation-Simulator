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
| [ARCHITECTURE.md](ARCHITECTURE.md) | Components, data flow, simulation tiers and their invariants, interfaces, known interface gaps | docs |
| [DOCS_STANDARD.md](DOCS_STANDARD.md) | What every component's docs must cover; PR checklist | docs |

## Components

| Component | Doc | What it covers |
|---|---|---|
| sim-core | [crates/sim-core/README.md](../crates/sim-core/README.md) | Travel model library; `ScenarioInput` input schema (contract with the pipeline); `Sim` API |
| data-pipeline | [crates/data-pipeline/README.md](../crates/data-pipeline/README.md) | `tsim-data`: Census, LODES and OSM to zones, flows, roads, transit. Inputs, outputs, known data gaps |
| app | [app/README.md](../app/README.md) | Tauri shell and map UI |

Component links resolve once each component's first PR is merged to `main`.

## For agent developers

[CLAUDE.md](../CLAUDE.md) at the repo root points agents at these docs and the rules they must follow.
