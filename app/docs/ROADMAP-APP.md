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
  a cash/budget line from sim-core's ledger (F16, F23, F26) (#22)
- Sim-owned clock: shell worker drives `Sim::tick`; client polls and
  interpolates; skip to next month; ridership refreshes after each daily re-run (F21) (#23)
- Real Travis County data by default (`npm run data`, then the shell finds the
  downloaded build); panel names the scenario source (#26)
- Construction period on: lines under construction draw faded with their
  opening day in the panel and the drawing hint (F24) (#27)
- Real Red Line preset: OSM route 550 stations, $3.50 fare, 32/51-minute
  peak/off-peak headways (`offpeakHeadwayMin`); about 640 riders/day alone on an empty map
  since SIM-10 (APTA 1,500; sim-core 1,917 inside today's network) (#31, #38)
- Monthly statements: last two closed months under the budget (F26)
- Line inspector: boardings by stop, segment loads both ways, peak load vs capacity (F17)
- Bus lines drawn along the streets from sim-core's `segment_shapes`, draft included (SIM-8, A29)
- Bond menu capped at borrowing capacity; financing warning in the build preview (SIM-9, A30)
- Bonds (issue from the budget panel), debt and payments, distress count and
  bankruptcy banner (F26)

## Next, M0/M1 (blocked items say on what)

| Item | Blocked on |
|---|---|
| M1 exit check in the UI: Red Line replica vs reported ridership | calibrated mode split (F14) and real scenario |

## M2 (money, clock, agents)

- Local-tier agents near the camera via deck.gl; above 10x draw flow animations

## M3 (statewide)

- Stream statewide tract-level layers; regional tier per metro
- Schematic at State zoom for intercity lines (needs a smaller minimum spacing or a global layout)

## M4 (MVP polish)

- Station pinning/nudging that survives re-layout
- Save/load, onboarding scenario
- Bundled offline tiles (PMTiles), tightened CSP, real icons
