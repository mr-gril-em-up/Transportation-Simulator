# App architecture

## Shape

```
┌──────────────────────── Tauri window (src-tauri, Rust) ───────────────────────┐
│  Webview: Vite-built TypeScript client                                        │
│                                                                               │
│   index.html HUD ──events──► main.ts ──► SimClient (sim/api.ts)               │
│        ▲                       │   ▲         │                                │
│        │ renderPanel()         │   │         ├─ MockSim (sim/mock.ts)  [now]  │
│        │                       │   │         └─ TauriSim → invoke() →        │
│        │                 render()  │            Rust command → sim-core [next]│
│        │                       ▼   │                                          │
│   MapLibre map (map.ts) ◄── GeoJSON sources ◄── schematic.ts (rail layout)    │
│   draw.ts (clicks → stops)  ─────────► main.addRoute()                        │
│   clock.ts (rAF loop, independent of zoom)                                    │
└───────────────────────────────────────────────────────────────────────────────┘
```

The client holds all UI state. The simulation is reached only through the
`SimClient` interface. Rendering is a pure function of `routes`, `results` and
the current zoom: every change calls `render()` which rebuilds the GeoJSON for
the line and stop sources. With M1-sized networks (tens of lines) that costs
well under a millisecond, so there is no diffing or caching.

## Modules

| File | Responsibility | Depends on |
|---|---|---|
| `sim/api.ts` | The contract: `Mode`, `Stop`, `RouteSpec`, `RouteResult`, `SimClient`, `isRegional()` | nothing |
| `sim/index.ts` | `createSim()`, the only place that picks a backend | api, mock |
| `sim/mock.ts` | `MockSim` and the stand-in model ([MOCK-SIM.md](MOCK-SIM.md)) | api |
| `map.ts` | Creates the MapLibre map, loads the basemap (with offline fallback), adds every source and layer, defines zoom bands | maplibre-gl, data/geo |
| `schematic.ts` | Octilinear layout of regional lines, interchange merging, morph helper ([SCHEMATIC.md](SCHEMATIC.md)) | api, mock.km |
| `draw.ts` | Drawing state machine: click adds a stop, Enter/double-click finishes, Esc cancels, snap to existing stops | map |
| `clock.ts` | `GameClock`: game minutes, pause, time-lapse steps | nothing |
| `main.ts` | Wires everything: route list, sim round trips, render, HUD events, keyboard, clock loop | all of the above |

## How the map and the sim talk

1. The player finishes a line in `draw.ts`, which calls back into
   `main.addRoute(mode, points)`.
2. `addRoute` builds a `RouteSpec` (ids, default headway 15 min bus / 30 min
   rail, fare $1.25, a color) and calls `sim.upsertRoute(route)`.
3. The sim returns a `RouteResult[]` for **every** route, not only the edited
   one. Lines compete and share demand once the real model lands, so an edit to
   one line can change another's ridership; returning the full set keeps the
   client from caching stale numbers.
4. `apply()` stores the results, records the round-trip time in the panel
   (`sim N ms`), and calls `render()` and `renderPanel()`.
5. `render()` puts per-stop boardings into the `stops` source; the stop
   circle radius and labels read them (`boardings` property) at City zoom and
   closer.

Every `SimClient` method returns a `Promise`. The mock resolves immediately,
but the real backend crosses the Tauri IPC boundary, so the UI is already
written for async results.

### The contract (`sim/api.ts`)

```ts
interface SimClient {
  upsertRoute(route: RouteSpec): Promise<RouteResult[]>; // add or replace, re-run, all results
  removeRoute(id: string): Promise<RouteResult[]>;
}
interface RouteSpec { id; name; mode: Mode; color; stops: Stop[]; headwayMin; fare }
interface RouteResult { routeId; dailyRiders; boardingsByStop: Record<stopId, number> }
```

`Mode` mirrors the mode list of sim-core's `TransitLine` in
[`crates/sim-core/src/schema.rs`](../../crates/sim-core/src/schema.rs) exactly
(no hyperloop), so a `RouteSpec` maps onto a `TransitLine` field by field:

| App `RouteSpec` | sim-core `TransitLine` |
|---|---|
| `name` | `name` |
| `mode` | `mode` (same strings) |
| `stops[].lngLat` | `stops[].lon`, `stops[].lat` |
| `stops[].name` | `stops[].name` |
| `headwayMin` | `headway_min` |
| `fare` | `fare` |
| `id`, `color` | app-only (the sim keys lines by whatever id its API returns) |

### Swapping in sim-core

sim-core's public `Sim` API (build from scenario, add/update/remove lines,
read results, advance the clock) lands in a later sim-core PR. When it does:

1. Add `sim-core = { path = "../../crates/sim-core" }` to
   `src-tauri/Cargo.toml` (the line is there, commented).
2. In `src-tauri/src/main.rs`, hold a `Mutex<sim_core::Sim>` in Tauri state
   and expose `upsert_route` / `remove_route` commands that convert
   `RouteSpec` to `TransitLine` and return results.
3. Add `src/sim/tauri.ts` with `class TauriSim implements SimClient` calling
   `invoke("upsert_route", { route })`.
4. Change `createSim()` to return `TauriSim` when running inside Tauri
   (`"__TAURI_INTERNALS__" in window`) and `MockSim` in a plain browser, so
   `npm run dev` keeps working without Rust.

No other file changes. If sim-core's results carry more than ridership
(travel times, loads, costs), extend `RouteResult`; the UI ignores fields it
does not render.

Clock ownership: today the clock lives in the client because nothing in the
mock depends on time of day. When sim-core owns the shared clock ("advance the
shared clock" in its planned API), `GameClock.tick()` will call the sim's
advance with `realSeconds * speed` game minutes instead of keeping its own
count. The UI contract (pause, time-lapse button, zoom-independent speed) does
not change.

## Map layers (`map.ts`)

Drawn bottom to top over the basemap:

| Layer | Source | Purpose |
|---|---|---|
| `wash` | world polygon | White overlay fading in below z11.5 so the schematic reads cleanly over the basemap ("roads fade to a thin layer underneath") |
| `outlines` | Texas + Travis County outlines | Orientation, and the only geography when the basemap is offline |
| `bus-lines` | `bus-lines` | Local modes (bus, trolleybus, BRT), `minzoom` 11 |
| `rail-lines` | `rail-lines` | Regional modes, coordinates morphed between schematic and geography |
| `stops` | `stops` | White dots; interchange stations get a black ring; radius grows with boardings at City zoom |
| `stop-labels` | `stops` | `Name · boardings`; regional stops from z8, local stops from z13 |
| `draft-line`, `draft-stops` | `draft` | The line being drawn |

Zoom bands follow the design doc's table (view width to zone unit):
State below z7.5, Region to z10.5, City to z13.5, Neighborhood above.
Schematic is full at z10 and below, geography full at z11.5 and above, and
the morph runs in between.

### Basemap

Default style: OpenFreeMap "liberty" (free, keyless, OSM-based). Override with
`VITE_MAP_STYLE=<style url>` in `app/.env.local`. The style is fetched with a
4-second timeout; on failure the app uses a plain background style plus the
built-in outlines, so the game still runs offline and in sandboxed CI. Labels
need the glyph server; offline they silently don't render.

OSM data is ODbL: MapLibre's attribution control is on (compact) and must stay.

## The clock (`clock.ts`)

`GameClock.minutes` counts game minutes since Day 1 00:00 (starts at 06:00).
`main.ts` runs a `requestAnimationFrame` loop that calls
`tick(realSecondsSinceLastFrame)`. `tick` adds `realSeconds * speed` unless
paused. Speed comes only from the time-lapse steps `[1, 2, 10, 60]`. Nothing
in the zoom path touches the clock: `map.on("zoom")` only calls `render()`.
This is the "zoom never changes the timescale" rule, and the unit test
`clock > runs 1 game minute per real second...` pins the arithmetic.

Skip-to-next-month and monthly budget settlement are M2 work.

## Desktop shell (`src-tauri/`)

Minimal Tauri 2 app: one window (1400x900, min 900x600) loading the Vite dev
server in dev (`devUrl`, port 1420) and `../dist` in builds. `csp` is null
for now because the basemap pulls tiles and fonts from a third-party host;
tighten it when tiles are bundled locally. Capabilities grant only
`core:default`. Icons were generated with `npx tauri icon` from a placeholder
(red circle with a white T); replace `icons/` by re-running that command on
real art.

## Testing

- `npm test` runs [test/core.test.ts](../test/core.test.ts): clock arithmetic,
  schematic invariants (octilinear elbows, one schematic point per stop,
  interchange merging), and mock-sim sanity (Red Line ridership in a plausible
  band, ridership rising with frequency).
- Manual / e2e: `npm run dev`, then exercise the loop. In dev builds the map is
  exposed as `window.__map` so scripts (Playwright) can set zoom directly.
