# Decision log

Decisions that shape the code, with the reason behind each. Anyone (human or agent) who makes or reverses a decision that a future developer would otherwise have to ask about adds a row in the same PR.

Status values: **Decided** (by Collins), **Approved** (proposed in the design doc and approved by Collins), **As built** (made during implementation; rationale recorded by the implementer).

## How to add a decision

IDs are per stream so parallel PRs never pick the same number:

| Prefix | Stream | Section |
|---|---|---|
| `SIM-` | `crates/sim-core` | [Simulation core](#simulation-core-sim) |
| `DATA-` | `crates/data-pipeline` | [Data pipeline](#data-pipeline-data) |
| `APP-` | `app/` | [App](#app-app) |
| `DOC-` | Project-level decisions (scope, licence, process) and `docs/` | [Project and docs](#project-and-docs-doc) |

1. Take the next number in your stream's section (`SIM-3` after `SIM-2`).
2. **Append the row at the bottom of that section.** Sections run oldest first, so two PRs in different streams never touch the same lines.
3. Cite it as `SIM-3` in code comments and docs.
4. To reverse a decision, add a new row and set the old row's Status to "Superseded by SIM-7".

If two PRs in the same stream still pick the same number, the one that merges second renumbers before merging.

`D1` to `D31` below are the original single-sequence IDs. They are frozen: keep citing them as they are, never renumber them, and never add new `D` rows.

## Simulation core (SIM)

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| SIM-1 | 2026-11-27 | Lines build before opening: `build_years` per mode / `construction_speedup` (light rail 6 years, ~183 game days). Capital and the grant decision happen at the start; a realignment keeps the old version running; service changes apply at once; `instant_build` turns construction off and defaults to true until the app shows pending lines | The approved 12x compression (D9) only matters if building takes time; charging up front on the forecast mirrors FTA rating projects before construction; keeping the old line running avoids punishing extensions | As built | [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#construction-period-f24) |
| SIM-2 | 2026-11-27 | sim-core simplifies the road graph itself at load: dead-end trees without a zone node are dropped and degree-2 chains are merged into edges that keep their links; a chain's BPR time is the sum of its links' BPR times | Exact (zone-to-zone times unchanged, tested on real Travis); halves load and daily run time on the real build (10.3 s to 5.8 s, 3.5 s to 1.9 s); the pipeline keeps emitting plain intersection-split links | As built | [sim-core ARCHITECTURE.md](../crates/sim-core/docs/ARCHITECTURE.md#threading-and-performance) |

## Data pipeline (DATA)

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| DATA-1 | 2026-11-27 | Pipeline builds are distributed as GitHub Release assets (`data-<area>-<year>`), published by the manual data-pipeline workflow; never committed to git | Consumers (app, sim-core, reviewers) get real data with one command and no raw downloads; keeps tens of MB of generated JSON out of history; re-runs replace assets in place | As built | [pipeline README](../crates/data-pipeline/README.md#use-a-published-build-no-raw-downloads) |

## App (APP)

Detailed app decisions also live in [app/docs/DECISIONS.md](../app/docs/DECISIONS.md); list here the ones other streams need to know.

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| APP-1 | 2026-11-27 | sim-core owns the game clock: a worker thread in the desktop shell calls `Sim::tick` every 100 ms under the Sim lock; the client polls `clock` and interpolates | One clock for every tier; polling needs no event plumbing and the day number tells the client when to re-read results | As built | [app A23](../app/docs/DECISIONS.md) |
| APP-2 | 2026-11-27 | The desktop app opens on the pipeline's published Travis build by default: `npm run data` runs `crates/data-pipeline/scripts/download-build.sh`, and the shell loads `crates/data-pipeline/data/out/48453-2023/scenario.json` if present (after `TRANSPO_SCENARIO`, before the synthetic region) | Real roads and commutes in every dev checkout with one command; reuses DATA-1's release instead of committing data; the synthetic fallback keeps CI offline | As built | [app A24](../app/docs/DECISIONS.md) |

## Project and docs (DOC)

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| DOC-1 | 2026-11-27 | Decision IDs are per stream (`SIM-`, `DATA-`, `APP-`, `DOC-`), appended at the bottom of the stream's section; `D1`–`D31` frozen | One shared sequence made parallel PRs collide on numbers and on the same table lines (#6, #7, #8, #16) | As built | Process fix requested by the coordinator |

## Legacy decisions D1–D31 (frozen, newest first)

| # | Date | Decision | Why | Status | Source |
|---|---|---|---|---|---|
| D31 | 2026-11-27 | Capital is charged when a line is added or extended, only for cost above what the line already paid; shortening or removing refunds nothing. `preview_line` prices a line by evaluating it and removing it again | No construction period yet (F24), so charging at the edit is the simplest honest rule; sunk construction cannot be resold. Reusing the line-edit path makes the preview's ridership and grant exactly what building would give | As built | [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#10-capital-cost-and-funding) |
| D30 | 2026-11-27 | The network is funded by federal and state grants plus a local tax base: federal capital grants (50%) for lines under a capital-per-daily-rider gate, operating assistance as a share of operating cost, and a 1% local sales tax over the population a network serves; settled monthly | Mirrors how Texas transit is paid for (FTA CIG, formula funds, CapMetro/DART/METRO 1% sales tax), and makes growing coverage grow the tax base | Decided | Collins; [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#10-capital-cost-and-funding) |
| D29 | 2026-11-27 | sim-core defaults use the sourced figures: 2.28 total trips per person per day (non-work = total minus work, floored at 0), NTD 2023 cost per car revenue hour with a cars-per-vehicle count (bus 182, light rail 436 x 2, commuter rail 716 x 2), CapMetro fares; a test checks them against `figures.csv` | Collins chose the real sourced figures over the design doc's numbers; NTD reports rail hours per car, so storing per-car cost keeps the default equal to the published row | Decided | Collins; [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#2-trip-generation) |
| D28 | 2026-11-27 | sim-core's daily re-run is warm-started from the previous day's road volumes (one MSA round), and each feedback round builds one shortest-path tree per origin used for skims, mode choice and loading together | Cut the Travis-size daily run from 13 s to 2 s and load from 15 s to 7 s with the same equilibrium (tests check within 1–3%); the strategic tier only needs to absorb the day's line edits | As built | [sim-core ARCHITECTURE.md](../crates/sim-core/docs/ARCHITECTURE.md#threading-and-performance) |
| D27 | 2026-11-27 | One CI workflow (`rust.yml`) checks the whole root Cargo workspace on PRs and main: fmt, build, test, clippy and doc with warnings as errors, plus the sim-core benchmark that fails if a line edit takes 2 s or more | Per-crate workflows missed sim-core entirely; the bench turns the F18 performance target into a gate so regressions show up in review | As built | [rust.yml](../.github/workflows/rust.yml) |
| D26 | 2026-11-27 | The Tauri shell owns the `Sim` and maps client route ids to `LineId`; the webview reaches it only through `upsert_route`/`remove_route` commands that return results for every line | Keeps simulation state out of the webview (architecture invariant) and one id space in the UI; lines compete for riders, so any edit can change every line | As built | [app decisions A12 to A16](../app/docs/DECISIONS.md) |
| D25 | 2026-11-27 | sim-core constants are `Params` defaults with their source in a comment; where a D18 reference figure exists, the default copies it rather than reading the CSV at run time | Keeps sim-core headless with no pipeline dependency (repo rule); a sim-core test will check defaults against `figures.csv` so they cannot drift | As built | [sim-core README](../crates/sim-core/README.md#design-and-rationale) |
| D24 | 2026-11-27 | Mode choice is a six-way multinomial logit (drive, carpool, transit-walk, park-and-ride, walk, bike), not nested | Nesting parameters cannot be calibrated before real data; MNL error is small in car-dominated Texas | As built | [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#6-mode-choice) |
| D23 | 2026-11-27 | Car assignment is one AM peak hour, method of successive averages with mode-choice feedback (4 rounds), BPR link times | Same equilibrium as Frank-Wolfe without a line search; mode choice sits in the same loop; the peak hour is where congestion shapes commute choice | As built | [sim-core MODEL.md](../crates/sim-core/docs/MODEL.md#7-car-assignment-and-feedback) |
| D22 | 2026-11-27 | A transit line edit re-runs transit paths, mode choice and transit loading only; car times update at the next game-day boundary (`car_times_stale` flags the gap) | Gives a ~20 ms line edit; car volumes move a few percent per line; matches the strategic tier's once-per-day cadence | As built | [sim-core ARCHITECTURE.md](../crates/sim-core/docs/ARCHITECTURE.md#what-is-recomputed-when) |
| D21 | 2026-11-27 | The game clock lives in sim-core (`Clock`, 1 real s = 1 game min at 1x); `Sim::tick` re-equilibrates once when one or more game days pass | One source of time for every tier (D4); at 60x a day is 24 real s, enough for an equilibrium | As built | [sim-core ARCHITECTURE.md](../crates/sim-core/docs/ARCHITECTURE.md#the-clock) |
| D20 | 2026-11-27 | sim-core uses `rayon` for per-origin parallel loops | Every hot loop is independent per origin zone; rayon is the standard pure-Rust choice | As built | [sim-core ARCHITECTURE.md](../crates/sim-core/docs/ARCHITECTURE.md#threading-and-performance) |
| D19 | 2026-11-27 | Zones connect to the nearest motorway/trunk-free road node in the largest connected component | Freeway connectors make access free; clipping fragments would leave a zone with no paths while still passing `validate()` | As built | [pipeline ARCHITECTURE step 6](../crates/data-pipeline/docs/ARCHITECTURE.md) |
| D18 | 2026-11-27 | Balance and calibration figures live in `crates/data-pipeline/reference/figures.csv`, one row per figure with source URL, dollar year and status (verified, derived, unverified) | Replaces the design doc's from-memory numbers with traceable ones; a test keeps every row cited | As built | [REFERENCE_FIGURES](../crates/data-pipeline/docs/REFERENCE_FIGURES.md) |
| D17 | 2026-11-27 | Repo made public with no licence; root `LICENSE` file states copyright and all rights reserved, and excludes ODbL OSM data. Holder name pending (Collins); added in a follow-up PR | Public visibility is not permission to reuse; an explicit notice removes doubt | Decided | Collins |
| D16 | 2026-11-27 | The repo stays closed: `UNLICENSED`, all rights reserved. All crates inherit the workspace licence | Not open source for now; revisit before any public release | Decided | Collins |
| D15 | 2026-11-27 | The Texas MVP is a sandbox: no region unlocks, no scenario win conditions. Both move to post-MVP. Onboarding (F45) stays as a guided session without a win condition | Keeps MVP scope on the core question (is building against real statewide demand fun?) | Decided | Collins |
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
