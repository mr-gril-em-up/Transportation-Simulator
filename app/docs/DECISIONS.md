# App decisions

Choices made inside `app/`, newest last. Each has the reason and what it costs.
Project-level decisions (Texas MVP, Rust core, one clock, tube map, all
modes except hyperloop) live in the design doc and the root docs.

### 1. Vanilla TypeScript, no UI framework
The HUD is four small panels. React/Svelte would add a build layer and a
state model for no gain at this size. Cost: if the HUD grows into menus,
budgets and dialogs (M2), revisit; `renderPanel()` string templates won't
scale far.

### 2. MapLibre GL only; deck.gl deferred
The design doc proposes MapLibre + deck.gl. M0/M1 draw tens of lines and
hundreds of stops, which MapLibre GeoJSON layers handle easily. deck.gl
earns its place with the local tier (20,000 moving agents at 60 fps, M2).
Add it then, as an overlay on the same map.

### 3. Sim reached only through `SimClient`, mock in TypeScript
The real sim is Rust (sim-core) and will run natively behind Tauri commands.
Putting the mock in TypeScript instead of Rust means `npm run dev` works in a
plain browser with no Rust toolchain, and the swap is one new file plus one
line in `createSim()`. Cost: the mock's math is throwaway.

### 4. `Mode` copies sim-core's mode strings
`RouteSpec.mode` uses the exact `TransitLine` mode names, so conversion to
sim-core is field renaming only. The UI groups them into local (bus,
trolleybus, BRT) and regional (rest) via `isRegional()`, which drives both
rendering (geographic vs schematic) and the mock's parameters.

### 5. Upserts return results for every route
Lines will compete for riders in the real model, so any edit can move every
line's numbers. Returning the full set avoids stale per-line caches.

### 6. Straight lines between stops
Bus routes should follow streets, which needs the road graph from the data
pipeline (and a router, probably in sim-core). Until then, lines are drawn
stop to stop. Ridership in the mock only depends on stop positions, so the
loop is still honest about what drives the numbers.

### 7. Bus lines hidden below City zoom (z11)
From the design doc: local routes stay geographic and only appear from City
zoom down, which keeps the schematic readable at Texas scale. The default
start zoom is 11.5 so a freshly drawn bus line is visible.

### 8. Schematic recomputed every zoom frame
The minimum station spacing is in screen pixels, so the layout depends on
zoom. Recomputing is sub-millisecond at M1 sizes. If statewide networks
make it slow, cache layouts per integer zoom and per network revision.

### 9. Client owns the clock for now
Nothing in the mock depends on time of day. When sim-core exposes "advance
the shared clock", `GameClock.tick()` drives it instead. The UI rules don't
change: one clock, time-lapse is its own button, zoom never touches speed.

### 10. Basemap from OpenFreeMap with an offline fallback
Free, keyless, OSM-based vector tiles, so no account or secret is needed to
run the game. Network failure falls back to a plain style with Texas and
Travis outlines so the app (and CI) still runs. Cost: a shipping game should
bundle its own tiles (PMTiles in the app package); that also lets the CSP be
tightened.

### 11. `src-tauri` is its own Cargo workspace
The root workspace (`members = ["crates/*"]`) belongs to sim-core. An empty
`[workspace]` in `app/src-tauri/Cargo.toml` stops Cargo from treating the app
as a stray member and keeps a separate `Cargo.lock` for the shell. sim-core
joins as a path dependency.
