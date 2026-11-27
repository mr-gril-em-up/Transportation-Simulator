# Tube-style schematic

Design doc rule: at State and Region zoom, regional transit switches from
geographic lines to a tube-style schematic (0/45/90 degree angles, spaced
stations, a color per line, interchanges as rings) and morphs back to
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
3. **Thin stops** (statewide view). The minimum gap between stations is 34 px
   on screen (`MIN_PX`). A line keeps its ends, its interchanges, and every
   stop at least one gap from the last stop kept; the rest are left out at
   this zoom, as a tube overview does. Each drawn stop carries `idx`, its
   index in the route, so the stop editor still finds it. Fully geographic
   (`t = 1`), `main.ts` lays out with an infinite scale, so every stop shows.
   Because the gap is in pixels, the layout is recomputed on every zoom frame
   (cheap: a few dozen lines, hundreds of stops).
4. **Walk toward the real stops.** The first stop sits at its real position.
   Each next hop goes in the direction from the previous *placed* stop to the
   next stop's real position, snapped to the nearest of 8 directions
   (`snapDir`), with length max(that distance, one gap). Aiming at the real
   position each hop keeps snapping error from building up: an intercity line
   with 300 km hops stays in its corridor instead of drifting off the state.
   Stations are therefore spaced as in reality above the minimum gap, not
   evenly (the earlier version walked an evenly spaced free layout and pushed
   Amtrak lines into Mexico at State zoom).
5. **Reuse placed stations.** A stop whose station an earlier line already
   placed reuses that position. When a hop lands on such a station at a
   non-octilinear angle, `elbow()` inserts one bend: a 45 degree leg for the
   shorter axis, then a straight leg.
7. **Morph pairing.** For each hop, the geographic polyline gets the same
   number of vertices as the schematic one (points placed at the same
   fraction of length), so `morph(schem, geo, t)` can lerp vertex by vertex.

`render()` in `main.ts` computes `t = clamp((zoom - 10) / 1.5, 0, 1)` and
draws `morph(schem, geo, t)` for lines and stops. `t = 0` is pure schematic,
`t = 1` pure geography.

## Known limits (accepted for M0/M1)

- Lines are laid out greedily. Two lines that run in parallel without sharing
  stations can cross or overlap; there is no edge-bundling or line offsetting.
- At State zoom a metro's lines collapse to a tangle a few gaps wide (DFW's
  DART, TRE, TEXRail and A-train); they read from Region zoom (about z8.5).
  See [statewide-dfw.png](img/statewide-dfw.png).
- Stations are not evenly spaced; long hops keep their real length.
- No player pinning/nudging of stations yet (the doc wants manual pins that
  survive re-layout). Planned: store pins per station id and treat them as
  pre-placed in step 5.
- If the heuristic reads badly on real networks, the doc's fallback is
  smoothed geographic lines: set `SCHEMATIC_FULL` below `minZoom` in
  `map.ts` and the morph never leaves geography.

A real optimizer (LOOM-style) can replace `layoutSchematic` without touching
callers: it only has to return the same `LineLayout[]`.
