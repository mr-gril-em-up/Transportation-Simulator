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

### A9. Client owns the clock for now (superseded by A23)
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

### A16. Dependencies optimized in dev builds
`[profile.dev.package."*"] opt-level = 3`: every dependency, not only
sim-core. Debug sim-core built the synthetic scenario in 16 s, which made
`npm run tauri dev` feel broken; optimizing only sim-core and rayon-core
still left serde_json debug, and real Travis startup took about 20 s.
With all dependencies optimized it is about 11 s. The shell crate stays
unoptimized for fast rebuilds. Cost: the first dev build takes about 5
minutes (CI caches `target/`).

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

### A22. Cost preview in the drawing hint, budget in the ridership panel
The preview appears where the player is already looking while placing stops,
and updates per stop, so the cost of one more station is visible before it is
placed. It goes through the serial sim queue rather than a separate channel
because sim-core's `preview_line` temporarily mutates the Sim; a generation
counter keeps a burst of clicks from queueing stale previews. Stop edits are
charged on release without a preview: a drag already shows ridership after
release, and the budget line shows the cost change. If players get surprised
by edit costs, add a preview on drag (F16 follow-up).

### A23. The sim owns the clock; the client polls it
A shell worker calls `Sim::tick` every 100 ms; the client polls `clock` every
500 ms and interpolates in between. Polling over Tauri events: the only thing
pushed would be "the day changed", which the client sees within half a second
from the polled `day`, and polling needs no `AppHandle` in `SimState`, so the
state stays testable without a running app. Ticking under the same lock as
edits keeps the one-`Sim`, one-writer model; the cost is that an edit at
midnight waits for the day re-run. sim-core's start time (day 0 05:00) is
adopted; the client's old 06:00 start is gone.

### A24. Real Travis data by default, from the pipeline's release, not bundled
The shell loads `TRANSPO_SCENARIO`, else the Travis build that `npm run data`
downloads to the pipeline's gitignored `data/out/48453-2023/`, else the
synthetic region. The path is resolved from the shell crate's source
directory, so it works in any checkout with no configuration, and the
download reuses the pipeline's own script (DATA-1) instead of a second copy.
Not committed (27 MB of JSON, DATA-1 keeps generated data out of git) and not
bundled yet (installers are not an M1 need). The synthetic fallback keeps a
fresh clone and CI working with no network. The panel names the source so a
synthetic run is never mistaken for real numbers.

### A25. Construction period on in the game, new lines drawn faded
SIM-1 left `instant_build` on until the app could show lines under
construction; with the panel line and faded drawing in place, the shell turns
it off. Faded (opacity) rather than dashed because MapLibre's `line-dasharray`
is not data-driven, and a second pair of layers for one state is more code
than it is worth. Rebuilds draw normally: a line the player can ride should not
look unbuilt. Their per-stop boardings are hidden until the rebuild opens,
because sim-core's `boardings_by_stop` is aligned with the running
version's stops, not the drawn ones; the line total still shows. The mock stays instant; it exists for UI work without Rust.

### A26. Bonds from a menu of sizes, deficit cover first
A free-form amount field invites typos worth billions; a short menu (cover
the deficit, $100M, $500M, $1B) covers what a player needs and the first
option is the usual answer. Rate and term are shown, not chosen: sim-core
fixes them from sourced figures. Bankruptcy is a banner, not a modal: the
sim keeps running (sim-core's choice) and the player should be able to look
at why. The shell computes debt and payment totals so the client never
re-implements bond math for sim-core's numbers.

### A27. Statements as a collapsed table in the budget panel
Two closed months side by side answer the usual question ("why did cash
drop?") without a separate screen; a modal or tab would hide the map the
player is changing. The table starts collapsed so the panel stays short.
The month in progress is not a column: sim-core books fares, operating cost
and tax only at settlement, so a partial column would show capital and no
income and mislead. Three columns overflow the 260 px panel. The client
keeps 12 months, enough for a year view later without changing the API.

### A28. Line inspector inline in the panel, charts as hand-written SVG
The inspector opens under its line in the ridership panel instead of in a
popup, so the map stays visible while the player changes headway or stops
and watches the loads move. The two charts are a few dozen lines of SVG
string building; a chart library would add a dependency and bundle size
for bars and a step line. Daily segment loads are drawn rather than
peak-hour ones because that is what sim-core reports per segment; the
peak-hour figure exists only for the busiest segment and is shown as text
against capacity.

### A29. Draw sim-core's street route; do not route in the app
Bus paths come from sim-core (`segment_shapes`, SIM-8) instead of an app
router. sim-core already holds the road graph and uses the path for length,
cost and travel time, so drawing its shape keeps the map and the score in
agreement and avoids shipping 93k road nodes over IPC. Agreed with the
sim-core thread before either side was built. Rail stays straight: rail
alignments are not modeled.

### A30. Bond menu capped at capacity; financing warning in the preview
Sizes sim-core would refuse are not offered, and the deficit-cover option
shrinks to the capacity ("all you can borrow"). The build preview warns,
but does not block, when capital after grant exceeds cash plus capacity:
sim-core charges capital regardless (SIM-9), and the line's own stops raise
capacity once built, so blocking would stop the first line on every map.

### A31. Existing lines are client routes with shell-chosen ids
The shell names lines present at start `e0..en` and maps them to sim-core's
LineIds, so the client edits, deletes, inspects and draws them through the
same paths as lines the player drew. A separate read-only layer would need
its own results mapping and would stop the player changing a real route.
They are grouped in a collapsed section because 61 rows would bury the
player's own lines.

### A32. The tier follows the scenario size
More than 2,000 zones runs sim-core's strategic tier with its recommended
settings (60-minute pair cutoff, 8 cold rounds). A county build has under
1,000 zones and the statewide tract build 6,896, so the threshold separates
them with room either way, and no setting has to be kept in step with the
file. 8 rounds over 4: sim-core measured 69 s and 33 km/h statewide peak
against 41 s and 12 km/h, and a congested state that is wrong costs more
than half a minute of loading.

### A33. Real rail the sim does not run is drawn, never scored
A statewide view with only Austin's network on it reads as an empty state,
and the player plans against the real DART and METRORail lines. The
pipeline's `transit.json` already has them (OSM), but no headways outside
CapMetro, so sim-core cannot run them yet. Drawing them as reference keeps
the score honest (no riders, no cost) and the panel says "drawn only".
They join the schematic layout so a drawn line meets them at real
interchanges. When the pipeline writes them to `existing_lines.json` with
headways, they move to today's network and this layer shrinks to nothing.

### A34. County detail is a view, not a second model
"Hand off to county detail" is the camera crossing into City zoom: the
schematic has morphed to geography, local lines and stops show, and the
badge names the county. The shell does not swap to the county's block-group
build, because the architecture's invariant is that the camera never changes
the score: two models would give two answers for the same line. A detailed
local model under the camera is the local and regional tiers (F28, F35).

### A35. A waiting command goes before the next day slice
With the day computed in slices (SIM-18), the clock worker and the commands
take turns on one Mutex, which is not fair: a worker that sleeps 100 ms and
then grabs the lock can keep landing ahead of an edit. A static counter of
waiting commands makes the worker skip its slice while any wait. It costs no
game time (the next step carries the elapsed time) and needs no change to
the lock or to sim-core. Measured statewide with 415 lines, the edit was
still about 16 s, the same as with no day running, so the remaining cost is
the edit itself, not the lock.
