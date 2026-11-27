// Tube-style schematic layout for rail lines, plus the geo<->schematic morph.
// Heuristic, not optimal: each line walks its stops, snapping every hop to a
// multiple of 45 degrees and aiming each hop at the stop's real position, so
// snapping error does not build up along a 600 km intercity line (statewide
// view). Hops are at least MIN_PX on screen. Stops of different lines within
// MERGE_KM become one interchange station; a line reaching an already placed
// station bends to it with one diagonal plus one straight leg.
import type { LngLat, RouteSpec } from "./sim/api";
import { km } from "./geo";

type XY = [number, number]; // lng, mercator-y in degrees: angles match the screen

const MERGE_KM = 0.3;
const MIN_PX = 34; // minimum on-screen gap between stations

const toXY = ([lng, lat]: LngLat): XY => [lng, (Math.log(Math.tan(Math.PI / 4 + (lat * Math.PI) / 360)) * 180) / Math.PI];
const toLngLat = ([x, y]: XY): LngLat => [x, (Math.atan(Math.exp((y * Math.PI) / 180)) * 360) / Math.PI - 90];
const sub = (a: XY, b: XY): XY => [a[0] - b[0], a[1] - b[1]];
const add = (a: XY, b: XY): XY => [a[0] + b[0], a[1] + b[1]];
const len = (a: XY) => Math.hypot(a[0], a[1]);
const lerp = (a: XY, b: XY, t: number): XY => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];

/** Unit vector of `v` snapped to the nearest of the 8 octilinear directions. */
export function snapDir(v: XY): XY {
  const a = Math.round(Math.atan2(v[1], v[0]) / (Math.PI / 4)) * (Math.PI / 4);
  return [Math.cos(a), Math.sin(a)];
}

/** Points strictly between a and b so that a->...->b uses only 0/45/90 degree legs. */
export function elbow(a: XY, b: XY): XY[] {
  const [dx, dy] = sub(b, a);
  const ax = Math.abs(dx), ay = Math.abs(dy);
  const eps = 1e-9;
  if (ax < eps || ay < eps || Math.abs(ax - ay) < eps) return [];
  const d = Math.min(ax, ay);
  return [[a[0] + Math.sign(dx) * d, a[1] + Math.sign(dy) * d]];
}

/** One regional line laid out both ways; `geo` and `schem` have equal length for the morph. */
export interface LineLayout {
  routeId: string;
  color: string;
  geo: LngLat[]; // same length as schem, for pointwise morph
  schem: LngLat[];
  /** Stops drawn at this zoom; `idx` is the stop's index in the route (zoomed out, some are left out). */
  stops: { id: string; idx: number; name: string; geo: LngLat; schem: LngLat; interchange: boolean }[];
}

/** pxPerDeg: screen pixels per degree of longitude at the current zoom. */
export function layoutSchematic(routes: RouteSpec[], pxPerDeg: number): LineLayout[] {
  const minSpacing = MIN_PX / pxPerDeg;
  // Merge nearby stops of different lines into stations.
  const stations: { geo: LngLat; placed?: XY; lines: Set<string> }[] = [];
  const stationOf = new Map<string, number>(); // stop id -> station index
  for (const r of routes) {
    for (const s of r.stops) {
      let idx = stations.findIndex((st) => !st.lines.has(r.id) && km(st.geo, s.lngLat) < MERGE_KM);
      if (idx < 0) idx = stations.push({ geo: s.lngLat, lines: new Set() }) - 1;
      stations[idx].lines.add(r.id);
      stationOf.set(s.id, idx);
    }
  }

  const ordered = [...routes].sort((a, b) => b.stops.length - a.stops.length);
  const out: LineLayout[] = [];
  for (const r of ordered) {
    if (r.stops.length < 2) continue;
    // Zoomed out, stops closer than the minimum gap would push a line far from its geography
    // (25 stops x 34 px is wider than Texas at State zoom). Keep the ends, interchanges and
    // stops at least one gap from the last kept one, as a tube overview does.
    const keep = r.stops
      .map((s, i) => i)
      .filter((i) => i === 0 || i === r.stops.length - 1 || stations[stationOf.get(r.stops[i].id)!].lines.size > 1);
    const kept: number[] = [];
    for (let i = 0; i < r.stops.length; i++) {
      const last = kept[kept.length - 1];
      if (keep.includes(i) || len(sub(toXY(r.stops[i].lngLat), toXY(r.stops[last].lngLat))) >= minSpacing) kept.push(i);
    }
    const rStops = kept.map((i) => r.stops[i]);
    const geoXY = rStops.map((s) => toXY(s.lngLat));
    const st = rStops.map((s) => stations[stationOf.get(s.id)!]);
    /** Next stop from `prev`: the snapped direction toward the stop's real position, at least one gap long. */
    const hop = (prev: XY, target: XY): XY => {
      const v = sub(target, prev);
      const d = snapDir(v);
      const l = Math.max(minSpacing, len(v));
      return add(prev, [d[0] * l, d[1] * l]);
    };

    // Walk the line; already placed stations pull the line to them via an elbow.
    const stopXY: XY[] = [];
    const path: XY[] = [];
    const geoPath: XY[] = [];
    for (let i = 0; i < geoXY.length; i++) {
      let p: XY;
      if (st[i].placed) p = st[i].placed!;
      else if (i === 0) p = geoXY[0];
      else p = hop(stopXY[i - 1], geoXY[i]);
      st[i].placed ??= p;
      if (i > 0) {
        const mids = elbow(stopXY[i - 1], p);
        const seg = [stopXY[i - 1], ...mids, p];
        let segLen = 0;
        const cum = [0];
        for (let k = 1; k < seg.length; k++) cum.push((segLen += len(sub(seg[k], seg[k - 1]))));
        for (let k = 1; k < seg.length; k++) {
          path.push(seg[k]);
          geoPath.push(lerp(geoXY[i - 1], geoXY[i], segLen ? cum[k] / segLen : 1));
        }
      } else {
        path.push(p);
        geoPath.push(geoXY[0]);
      }
      stopXY.push(p);
    }
    out.push({
      routeId: r.id,
      color: r.color,
      geo: geoPath.map(toLngLat),
      schem: path.map(toLngLat),
      stops: rStops.map((s, i) => ({
        id: s.id,
        idx: kept[i],
        name: s.name,
        geo: s.lngLat,
        schem: toLngLat(stopXY[i]),
        interchange: st[i].lines.size > 1,
      })),
    });
  }
  return out;
}

/** t = 0 fully schematic, t = 1 fully geographic. */
export function morph(a: LngLat, b: LngLat, t: number): LngLat {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
}
