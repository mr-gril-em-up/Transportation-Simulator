// Tube-style schematic layout for rail lines, plus the geo<->schematic morph.
// Heuristic, not optimal: each line walks its stops, snapping every hop to a
// multiple of 45 degrees with even spacing. Stops of different lines within
// MERGE_KM become one interchange station; a line reaching an already placed
// station bends to it with one diagonal plus one straight leg.
import type { LngLat, RouteSpec } from "./sim/api";
import { km } from "./sim/mock";

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
  stops: { id: string; name: string; geo: LngLat; schem: LngLat; interchange: boolean }[];
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
    const geoXY = r.stops.map((s) => toXY(s.lngLat));
    let geoLen = 0;
    for (let i = 1; i < geoXY.length; i++) geoLen += len(sub(geoXY[i], geoXY[i - 1]));
    const spacing = Math.max(minSpacing, geoLen / (geoXY.length - 1));

    // Free layout from the origin.
    const free: XY[] = [[0, 0]];
    for (let i = 1; i < geoXY.length; i++) {
      const d = snapDir(sub(geoXY[i], geoXY[i - 1]));
      free.push(add(free[i - 1], [d[0] * spacing, d[1] * spacing]));
    }
    // Anchor on the first already placed station, else on the geographic centroid.
    const st = r.stops.map((s) => stations[stationOf.get(s.id)!]);
    const anchorIdx = st.findIndex((x) => x.placed);
    const centroid = (pts: XY[]): XY => pts.reduce((a, p) => add(a, [p[0] / pts.length, p[1] / pts.length]), [0, 0] as XY);
    const shift = anchorIdx >= 0 ? sub(st[anchorIdx].placed!, free[anchorIdx]) : sub(centroid(geoXY), centroid(free));

    // Walk the line; already placed stations pull the line to them via an elbow.
    const stopXY: XY[] = [];
    const path: XY[] = [];
    const geoPath: XY[] = [];
    for (let i = 0; i < geoXY.length; i++) {
      let p: XY;
      if (st[i].placed) p = st[i].placed!;
      else if (i === 0) p = add(free[0], shift);
      else p = add(stopXY[i - 1], sub(free[i], free[i - 1]));
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
      stops: r.stops.map((s, i) => ({
        id: s.id,
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
