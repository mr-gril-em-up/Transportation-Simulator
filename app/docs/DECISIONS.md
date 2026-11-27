# App decisions

Choices made inside `app/`, newest last. Each has the reason and what it costs.
Project-level decisions (Texas MVP, Rust core, one clock, tube map, all
modes except hyperloop) live in the design doc and the root docs.

### A1. Vanilla TypeScript, no UI framework
The HUD is four small panels. React/Svelte would add a build layer and a
state model for no gain at this size. Cost: if the HUD grows into menus,
budgets and dialogs (M2), revisit; `renderPanel()` string templates won't
scale far.

### A2. MapLibre GL only; deck.gl deferred
The design doc proposes MapLibre + deck.gl. M0/M1 draw tens of lines and
hundreds of stops, which MapLibre GeoJSON layers handle easily. deck.gl
earns its place with the local tier (20,000 moving agents at 60 fps, M2).
Add it then, as an overlay on the same map.

### A3. Sim reached only through `SimClient`, mock in TypeScript (amended by A12)
The real sim is Rust (sim-core) and will run natively behind Tauri commands.
Putting the mock in TypeScript instead of Rust means `npm run dev` works in a
plain browser with no Rust toolchain, and the swap is one new file plus one
line in `createSim()`. Cost: the mock's math is throwaway.

### A4. `Mode` copies sim-core's mode strings
`RouteSpec.mode` uses the exact `TransitLine` mode names, so conversion to
sim-core is field renaming only. The UI groups them into local (bus,
trolleybus, BRT) and regional (rest) via `isRegional()`, which drives both
rendering (geographic vs schematic) and the mock's parameters.

### A5. Upserts return results for every route
Lines will compete for riders in the real model, so any edit can move every
line's numbers. Returning the full set avoids stale per-line caches.

### A6. Straight lines between stops
Bus routes should follow streets, which needs the road graph from the data
pipeline (and a router, probably in sim-core). Until then, lines are drawn
stop to stop. Ridership in the mock only depends on stop positions, so the
loop is still honest about what drives the numbers.

### A7. Bus lines hidden below City zoom (z11)
From the design doc: local routes stay geographic and only appear from City
zoom down, which keeps the schematic readable at Texas scale. The default
start zoom is 11.5 so a freshly drawn bus line is visible.

### A8. Schematic recomputed every zoom frame
The minimum station spacing is in screen pixels, so the layout depends on
zoom. Recomputing is sub-millisecond at M1 sizes. If statewide networks
make it slow, cache layouts per integer zoom and per network revision.

### A9. Client owns the clock for now
sim-core exposes `Sim::tick`, but it re-equilibrates at each game-day boundary
(seconds), which needs a worker loop in the shell and result events back to
the client. That is F21's move; until then `GameClock` keeps time in the
client and line edits refresh results directly. The UI rules don't
change: one clock, time-lapse is its own button, zoom never touches speed.

### A10. Basemap from OpenFreeMap with an offline fallback
Free, keyless, OSM-based vector tiles, so no account or secret is needed to
run the game. Network failure falls back to a plain style with Texas and
Travis outlines so the app (and CI) still runs. Cost: a shipping game should
bundle its own tiles (PMTiles in the app package); that also lets the CSP be
tightened.

### A11. `src-tauri` is its own Cargo workspace
The root workspace (`members = ["crates/*"]`) belongs to sim-core. An empty
`[workspace]` in `app/src-tauri/Cargo.toml` stops Cargo from treating the app
as a stray member and keeps a separate `Cargo.lock` for the shell. sim-core
joins as a path dependency.

### A12. Two backends: sim-core in the desktop app, mock in the browser
`createSim()` returns `TauriSim` when `__TAURI_INTERNALS__` is present and
`MockSim` otherwise. The desktop app always uses sim-core. Keeping the mock
means `npm run dev` and UI work in a plain browser still need no Rust
toolchain. Cost: two backends can disagree. The panel header names the backend
that answered, so numbers are never mistaken for sim-core's.

### A13. The shell owns the `Sim`, builds it off-thread, and maps client ids
`SimState` holds `Mutex<Inner>` + `Condvar`: the `Sim` is built on a background
thread at startup (seconds), commands are `async` so waiting never blocks the
main thread, and the client-string-id to `LineId` map lives in Rust so the
webview holds no simulation state (architecture invariant). Rejected: building
the `Sim` lazily on the first command (first draw would freeze for seconds),
and exposing `LineId`s to the client (two id spaces in the UI).

### A14. Synthetic region by default, real scenario via `TRANSPO_SCENARIO`
Until the pipeline's Travis `scenario.json` ships with the app, the shell
builds `sim_core::synth::generate(&SynthConfig::default())` (Travis-sized,
centered on downtown Austin). Pointing `TRANSPO_SCENARIO` at a pipeline output
switches to real data without a rebuild. A menu-driven scenario picker waits
for save/load (F43).

### A15. Regional lines get park-and-ride at every stop
sim-core enables drive-to-transit access only at stops with `park_ride`. Most
Texas suburban rail trips start with a drive (design doc), so regional lines
set it on every stop and local lines on none. Too generous downtown;
replaced by per-stop editing (F16).

### A16. sim-core optimized in dev builds
`[profile.dev.package.sim-core] opt-level = 3` (and `rayon-core`). Debug
sim-core built the Travis scenario in 16 s, which made `npm run tauri dev`
feel broken. The shell stays unoptimized for fast rebuilds.

### A17. Sim calls are serialized in the client
Responses carry full state, so overlapping calls could apply out of order.
A promise chain (`serialQueue`) is simpler than sequence numbers and also
right when a later-issued call finishes first. Cost: one slow call delays the
next. At about 25 ms per edit that doesn't matter. Revisit if `tick` results
share the same channel (F21).

### A18. Desire lines: top 300 pairs, width relative to the busiest pair
300 chords is about where Travis stops being readable. Relative width
reads the same on the synthetic and real scenarios, and statewide later.
Color encodes transit share, so drawing a line visibly changes the layer.
deck.gl stays deferred (A2): 300 GeoJSON lines are trivial for MapLibre.

### A19. Stop editing gestures: drag, click to insert, right-click to delete
These are direct manipulation on the map with no toolbar modes, so editing
needs no extra UI. A new stop goes at the position that adds the least line
length (`bestInsertIndex`), which is where a planner would put it in almost
every case and needs no segment picking. A drag previews locally and calls
the sim once on release, so 20+ mouse moves never queue 20 sim runs.
Right-click (not a modifier key) works the same on every platform; the
webview context menu is suppressed.

### A20. Editing always draws true geography
In schematic or morph view, stop positions on screen are not their real
coordinates, so a drag would write a wrong location. While a line is being
edited, `render()` uses `t = 1` (pure geography) at every zoom, and the
editor fits the camera to the line.

### A21. Interchange stops move together
Snapping while drawing puts two lines' stops at the same spot. Dragging
one of them moves every other line's stop within 10 m (`LINK_KM`), and
each touched line is re-simulated. The tolerance is there because snapped
coordinates come from rendered, tile-quantized geometry, so they are not
bit-identical. Splitting an interchange means deleting the stop on one line
and adding a new one.
