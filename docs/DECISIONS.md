# Decision log

Decisions that shape the code, with the reason behind each. Newest first. Anyone (human or agent) who makes or reverses a decision that a future developer would otherwise have to ask about adds a row in the same PR.

Status values: **Decided** (by Collins), **Approved** (proposed in the design doc and approved by Collins), **As built** (made during implementation; rationale recorded by the implementer).

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| D16 | 2026-11-27 | Zones connect to the nearest motorway/trunk-free road node in the largest connected component | Freeway connectors make access free; clipping fragments would leave a zone with no paths while still passing `validate()` | As built | [pipeline ARCHITECTURE step 6](../crates/data-pipeline/docs/ARCHITECTURE.md) |
| D15 | 2026-11-27 | Balance and calibration figures live in `crates/data-pipeline/reference/figures.csv`, one row per figure with source URL, dollar year and status (verified, derived, unverified) | Replaces the design doc's from-memory numbers with traceable ones; a test keeps every row cited | As built | [REFERENCE_FIGURES](../crates/data-pipeline/docs/REFERENCE_FIGURES.md) |
| D14 | 2026-11-27 | Data pipeline written in Rust, not Python | One toolchain (`cargo` only); builds `sim_core::ScenarioInput` so a schema change breaks the pipeline's compile, not the game at load; streams the statewide OSM extract fast; few dependencies, no GDAL. Cost: ad-hoc analysis is easier in Python | As built | [pipeline architecture](../crates/data-pipeline/docs/ARCHITECTURE.md) |
| D13 | 2026-11-27 | Pipeline zones are 2020 block groups (12-digit GEOID); LODES 8 blocks roll up by their first 12 digits | LODES 8 uses 2020 blocks, so no crosswalk is needed | As built | [data-pipeline README](../crates/data-pipeline/README.md) |
| D12 | 2026-11-27 | sim-core owns the input schema (`ScenarioInput`); the pipeline conforms to it | One source of truth for the contract between build time and run time | As built | [sim-core README](../crates/sim-core/README.md) |
| D11 | 2026-11-27 | Releases tagged `v0.1.0`..`v0.5.0` at milestone exits; v0.5.0 is the Texas MVP; 1.0 is the post-MVP mode set | Matches the design doc's MVP vs 1.0 phase split | Proposed | [ROADMAP.md](ROADMAP.md) |
| D10 | 2026-11-27 | Strategic tier for all of Texas computes one game day in under 20 s | Keeps up with 60x time-lapse (a game day lasts 24 real s at 60x) | Approved | Design doc |
| D9 | 2026-11-27 | Construction and permitting compressed 12x by default, realism setting 1x to 24x; bond terms and vehicle lifetimes compress by the same factor | Real rail timelines (4–10 yr) would mean hours of waiting per project | Approved | Design doc |
| D8 | 2026-11-27 | Travis County is the test bench for M0 and M1 only | One county re-runs in under 2 s, so the model can be tuned many times a day; Austin rail ridership is a calibration check | Approved | Design doc |
| D7 | 2026-11-27 | Aggregate simulation is the truth; agents exist only near the camera and never feed the score | ~100M daily person-trips cannot run as agents in real time | Approved | Design doc |
| D6 | 2026-11-27 | Every mode is in scope, including ones not built in the US. **No hyperloop** | Modes share one pattern (network, vehicles, fare), so each is mostly data | Decided | Collins |
| D5 | 2026-11-27 | Zoomed-out regional transit draws as a London-tube-style schematic | Readability at State and Region zoom | Decided | Collins |
| D4 | 2026-11-27 | One game clock at every zoom; time-lapse is a separate control | Zoom must not change what the player is watching happen | Decided | Collins |
| D3 | 2026-11-27 | Rust simulation core; browser-rendered map inside a desktop game app; Tauri as the shell | Tauri is Rust-native, so the core runs as native threads, not WebAssembly; installs like a desktop game, no server | Decided (Tauri approved) | Collins |
| D2 | 2026-11-27 | Realistic models; build and policy timelines may be accelerated for gameplay | Credibility with players who know the field, without hours of waiting | Decided | Collins |
| D1 | 2026-11-27 | MVP is all of Texas, single player. Multiplayer not planned | Texas is the proof of concept | Decided | Collins |
