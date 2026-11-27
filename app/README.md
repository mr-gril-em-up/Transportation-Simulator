# Transpo-Sim app

The desktop game client for Transpo-Sim: a [Tauri 2](https://tauri.app) shell
wrapping a browser map client (Vite + TypeScript + [MapLibre GL](https://maplibre.org)).
This directory is owned by the app build stream. It is **not** part of the root
Cargo workspace (see [Repo layout](#repo-layout)).

Current scope is milestones **M0/M1** on the Travis County test bench, as set in
the [design doc](https://claude.ai/artifact/Hjw86789MWbzev8BMVZBz8):

| Feature | Status | Where |
|---|---|---|
| Map that zooms from all of Texas down to neighborhood level | Done | [src/map.ts](src/map.ts) |
| Draw a bus line (click stops) and see ridership come back from the sim | Done, against a mock sim | [src/draw.ts](src/draw.ts), [src/sim/](src/sim/) |
| Draw a rail line; Red Line demo preset | Done | [src/main.ts](src/main.ts), [src/data/geo.ts](src/data/geo.ts) |
| Zoomed-out regional lines as a London Tube style schematic, morphing back to geography | Done (heuristic layout) | [src/schematic.ts](src/schematic.ts) |
| One game clock with a separate time-lapse button; zoom never changes speed | Done | [src/clock.ts](src/clock.ts) |
| Real simulation (crates/sim-core) | Waiting on sim-core's public API | swap point: [src/sim/index.ts](src/sim/index.ts) |
| Road-snapped routes, desire lines, real zones | Waiting on data pipeline output | see [docs/ROADMAP-APP.md](docs/ROADMAP-APP.md) |

![City view with the Red Line demo and a drawn bus line](docs/img/city-view.png)
![Region zoom: the same Red Line as a tube-style schematic](docs/img/region-schematic.png)

## Quick start

Prerequisites: Node 20+ and npm. For the desktop shell also Rust (stable) and
the [Tauri system dependencies](https://tauri.app/start/prerequisites/)
(on Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev`).

```bash
cd app
npm ci              # install exact versions from package-lock.json
npm run dev         # browser client at http://localhost:1420 (no Rust needed)
npm run tauri dev   # same client inside the desktop window
```

| Command | What it does |
|---|---|
| `npm run dev` | Vite dev server on port 1420 (fixed; Tauri expects it) |
| `npm run build` | Type-check (`tsc --noEmit`) then production build into `dist/` |
| `npm test` | Unit tests (Vitest): clock, schematic layout, mock sim |
| `npm run tauri dev` | Desktop app with hot reload |
| `npm run tauri build` | Installable desktop bundle |
| `cd src-tauri && cargo check` | Check the Rust shell without building the frontend |

One-line CI check: `cd app && npm ci && npm run build && npm test && (cd src-tauri && cargo check)`

## How to play the prototype

- **Draw bus line / Draw rail line**: click the map to drop stops; **Enter** or
  double-click finishes, **Esc** cancels. Clicking on an existing stop snaps to
  it, so two lines can share an interchange.
- **Add Red Line demo**: loads an approximate CapMetro Red Line (9 stations).
- **Ridership panel** (top right): weekday riders per line, re-computed on every
  change. Change the headway dropdown to see ridership respond. `sim N ms`
  shows the round-trip time of the last simulation call (M1 target: under 2 s).
- **Clock** (top center): pause with the button or **Space**; **Time-lapse**
  (or **T**) cycles 1x, 2x, 10x, 60x. At 1x one real second is one game minute.
- **Zoom badge** (bottom left): current zoom band (State, Region, City,
  Neighborhood), whether lines are drawn schematic or geographic, and jump
  buttons for Texas and Travis County.

Bus lines appear only from City zoom (z11) and closer; that is intentional
(design doc: local routes stay geographic and only show from City zoom down).

## Repo layout

```
app/
├── index.html            HUD markup (panels, buttons)
├── src/
│   ├── main.ts           App wiring: state, rendering, UI events, clock loop
│   ├── map.ts            MapLibre setup, basemap + offline fallback, all layers, zoom bands
│   ├── draw.ts           Click-to-place line drawing tool
│   ├── schematic.ts      Tube-map layout + geo/schematic morph
│   ├── clock.ts          Game clock and time-lapse
│   ├── style.css         HUD styling
│   ├── data/geo.ts       Texas/Travis outlines, bounds, Red Line preset
│   └── sim/
│       ├── api.ts        SimClient interface + shared types (the contract)
│       ├── mock.ts       MockSim: stand-in model until sim-core lands
│       └── index.ts      createSim(): the single swap point
├── test/core.test.ts     Vitest unit tests
├── src-tauri/            Rust desktop shell (own [workspace], not in root workspace)
└── docs/                 Deeper docs (below)
```

The root `Cargo.toml` workspace (owned by sim-core) has `members = ["crates/*"]`.
`app/src-tauri/Cargo.toml` declares an empty `[workspace]` table so Cargo
treats it as its own root and never tries to join or conflict with the root
workspace. sim-core will be added as a path dependency
(`sim-core = { path = "../../crates/sim-core" }`, already stubbed as a comment).

## Further docs

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): modules, data flow, how the map and the sim talk, and the sim-core swap plan.
- [docs/SCHEMATIC.md](docs/SCHEMATIC.md): the tube-map layout algorithm and the zoom morph.
- [docs/MOCK-SIM.md](docs/MOCK-SIM.md): the stand-in demand model, its formulas and calibration.
- [docs/DECISIONS.md](docs/DECISIONS.md): choices made in this directory, with the reason and the trade-off.
- [docs/ROADMAP-APP.md](docs/ROADMAP-APP.md): what the app does next and what it is waiting on.
