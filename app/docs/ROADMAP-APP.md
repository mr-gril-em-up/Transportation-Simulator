# App roadmap (M0 to Texas MVP)

App-side work only. The project-wide Gantt roadmap lives in the root
`docs/ROADMAP.md` (owned by the docs stream). Milestones follow the design
doc.

## Done

- #2: Tauri 2 shell + Vite/TypeScript/MapLibre client; Texas-to-neighborhood
  zoom; bus and rail drawing; Red Line demo; ridership panel; `SimClient` with
  a mock; tube schematic with zoom morph; game clock with time-lapse
- #5: Space no longer double-toggles pause on a focused button
- sim-core integration: `TauriSim` over `upsert_route` / `remove_route` /
  `sim_status`; Sim built at startup from `TRANSPO_SCENARIO` or the synthetic
  region; cost and farebox in the panel (#9)
- Desire-line layer (F06) from `desire_lines`; sim calls serialized; blocking
  waits moved to the blocking pool (#9 review follow-ups) (#12)
- Stop editing (drag, insert, delete) and line rename (F16) (#15, #19)
- Build cost preview while drawing (capital, grant, riders, operating cost) and
  a cash/budget line from sim-core's ledger (F16, F23, F26)

## Next, M0/M1 (blocked items say on what)

| Item | Blocked on |
|---|---|
| Ship the pipeline's Travis `scenario.json` as the default scenario | pipeline output path/packaging agreed |
| Drive `Sim::tick` from a shell worker; results-changed events to the client (F21) | nothing |
| Snap drawn bus routes to the road graph | road graph from the pipeline; router location (sim-core vs app) to agree |
| Line inspector: per-stop boardings chart, segment load profile, peak load vs capacity | nothing (fields already cross IPC) |
| M1 exit check in the UI: Red Line replica vs reported ridership | calibrated mode split (F14) and real scenario |

## M2 (money, clock, agents)

- Budget panel: monthly statements from `Ledger.history`, settlement as the sim clock crosses months
- Skip-to-next-month in the clock; clock driven by sim-core
- Local-tier agents near the camera via deck.gl; above 10x draw flow animations

## M3 (statewide)

- Stream statewide tract-level layers; regional tier per metro
- Schematic at State zoom for intercity lines (needs a smaller minimum spacing or a global layout)

## M4 (MVP polish)

- Station pinning/nudging that survives re-layout
- Save/load, onboarding scenario
- Bundled offline tiles (PMTiles), tightened CSP, real icons
