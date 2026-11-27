# Tube-style schematic

Design doc rule: at State and Region zoom, regional transit switches from
geographic lines to a tube-style schematic (0/45/90 degree angles, evenly
spaced stations, a color per line, interchanges as rings) and morphs back to
true geography between Region and City zoom so the player never loses a line.
The doc flags auto-layout as an unsolved problem in general and recommends a
fast heuristic recomputed when the network changes. This is that heuristic.

Code: [src/schematic.ts](../src/schematic.ts). Tests: `schematic` block in
[test/core.test.ts](../test/core.test.ts).

## Which lines

Only regional modes (`isRegional(mode)`: everything except bus, trolleybus,
BRT). Local modes stay geographic and hidden below z11.

## Coordinates

Layout runs in `XY = [longitude, mercator-y in degrees]`. In Web Mercator a
degree of longitude and a "degree" of mercator-y are the same number of
screen pixels, so 45 degrees in XY is 45 degrees on screen. Results convert
back to lng/lat for MapLibre.

## Algorithm (`layoutSchematic(routes, pxPerDeg)`)

1. **Merge stations.** Walk all stops of all lines. A stop within 300 m
   (`MERGE_KM`) of a stop on a *different* line joins that station. Stations
   on more than one line are interchanges.
2. **Order lines** longest first, so the trunk sets the frame and branches
   attach to it.
3. **Spacing** per line = max(average geographic stop spacing, 34 px on
   screen). The minimum keeps stations legible when zoomed out; above it the
   schematic keeps the line's real scale. Because the minimum is in pixels, the
   layout is recomputed on every zoom frame (cheap: a few lines, tens of
   stops).
4. **Free layout.** From the origin, each hop goes one `spacing` in the
   direction of the real hop snapped to the nearest of 8 directions
   (`snapDir`).
5. **Anchor.** If any station on the line was already placed by an earlier
   line, translate so that station lands on its placed position. Otherwise
   translate so the schematic's centroid sits on the geographic centroid: the
   line stays roughly where it really is.
6. **Walk and connect.** Place each stop: reuse its station's position if
   already placed, else previous position plus the free-layout hop. When a hop
   lands on a pre-placed station at a non-octilinear angle, `elbow()` inserts one
   bend: a 45 degree leg for the shorter axis, then a straight leg.
7. **Morph pairing.** For each hop, the geographic polyline gets the same
   number of vertices as the schematic one (points placed at the same
   fraction of length), so `morph(schem, geo, t)` can lerp vertex by vertex.

`render()` in `main.ts` computes `t = clamp((zoom - 10) / 1.5, 0, 1)` and
draws `morph(schem, geo, t)` for lines and stops. `t = 0` is pure schematic,
`t = 1` pure geography.

## Known limits (accepted for M0/M1)

- Lines are laid out greedily. Two lines that run in parallel without sharing
  stations can cross or overlap; there is no edge-bundling or line offsetting.
- At State zoom the 34 px minimum makes a metro line look larger than its
  county. That is normal tube-map distortion, but statewide lines (M3) will
  need a smaller minimum at low zoom or a proper global layout.
- No player pinning/nudging of stations yet (the doc wants manual pins that
  survive re-layout). Planned: store pins per station id and treat them as
  pre-placed in step 5.
- If the heuristic reads badly on real networks, the doc's fallback is
  smoothed geographic lines: set `SCHEMATIC_FULL` below `minZoom` in
  `map.ts` and the morph never leaves geography.

A real optimizer (LOOM-style) can replace `layoutSchematic` without touching
callers: it only has to return the same `LineLayout[]`.
