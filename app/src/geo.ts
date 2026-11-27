import type { LngLat } from "./sim/api";

/** Equirectangular distance in km; accurate to <1% at county scale. */
export function km(a: LngLat, b: LngLat): number {
  const kx = 111.32 * Math.cos(((a[1] + b[1]) / 2) * (Math.PI / 180));
  return Math.hypot((a[0] - b[0]) * kx, (a[1] - b[1]) * 110.57);
}

const same = (a: LngLat | undefined, b: LngLat) => !!a && Math.abs(a[0] - b[0]) < 1e-7 && Math.abs(a[1] - b[1]) < 1e-7;

/**
 * A line's drawn path: each segment's street route where `shape` has one,
 * else straight from stop to stop. sim-core starts and ends every segment
 * shape exactly on its two stops, so a segment whose ends no longer match
 * (a stop moved since the shape was computed) is drawn straight, as is
 * every segment when the shape count does not match the stops.
 */
export function routePath(stops: LngLat[], shape?: LngLat[][]): LngLat[] {
  if (!shape || shape.length !== stops.length - 1) return stops;
  const out: LngLat[] = [stops[0]];
  shape.forEach((seg, k) => {
    const fresh = seg.length >= 2 && same(seg[0], stops[k]) && same(seg.at(-1), stops[k + 1]);
    out.push(...(fresh ? seg.slice(1, -1) : []), stops[k + 1]);
  });
  return out;
}
