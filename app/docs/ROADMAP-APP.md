# App roadmap (M0 to Texas MVP)

App-side work only. The project-wide Gantt roadmap lives in the root
`docs/ROADMAP.md` (owned by the docs stream). Milestones follow the design
doc.

## Done (this PR)

- Tauri 2 shell + Vite/TypeScript/MapLibre client
- Texas-to-neighborhood zoom with zoom bands, Travis County focus
- Draw bus and rail lines; Red Line demo; ridership panel with headway control
- `SimClient` interface with a mock backend
- Tube-style schematic for regional lines with zoom morph
- Game clock with separate time-lapse button

## Next, M0/M1 (blocked items say on what)

| Item | Blocked on |
|---|---|
| `TauriSim`: real sim-core behind `SimClient` | sim-core public `Sim` API |
| Load the Travis County scenario (zones, desire lines layer: top OD flows) | data-pipeline output in `ScenarioInput` format |
| Snap drawn bus routes to the road graph | road graph from the pipeline; router location (sim-core vs app) to agree |
| Stop editing: drag, insert, delete stops; rename lines | nothing |
| Line inspector: per-stop boardings chart, load profile | richer `RouteResult` from sim-core |
| M1 exit check in the UI: Red Line replica vs reported ridership | sim-core |

## M2 (money, clock, agents)

- Budget HUD: capital cost preview while drawing, operating cost, monthly settlement
- Skip-to-next-month in the clock; clock driven by sim-core
- Local-tier agents near the camera via deck.gl; above 10x draw flow animations

## M3 (statewide)

- Stream statewide tract-level layers; regional tier per metro
- Schematic at State zoom for intercity lines (needs a smaller minimum spacing or a global layout)

## M4 (MVP polish)

- Station pinning/nudging that survives re-layout
- Save/load, onboarding scenario
- Bundled offline tiles (PMTiles), tightened CSP, real icons
