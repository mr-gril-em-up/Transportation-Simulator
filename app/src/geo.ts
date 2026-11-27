import type { LngLat } from "./sim/api";

/** Equirectangular distance in km; accurate to <1% at county scale. */
export function km(a: LngLat, b: LngLat): number {
  const kx = 111.32 * Math.cos(((a[1] + b[1]) / 2) * (Math.PI / 180));
  return Math.hypot((a[0] - b[0]) * kx, (a[1] - b[1]) * 110.57);
}

/**
 * A line's drawn path: each segment's street route where `shape` has one,
 * else straight from stop to stop. A shape that does not match the stops
 * (stale after an edit) is ignored.
 */
export function routePath(stops: LngLat[], shape?: LngLat[][]): LngLat[] {
  if (!shape || shape.length !== stops.length - 1) return stops;
  const out: LngLat[] = [stops[0]];
  shape.forEach((seg, k) => out.push(...(seg.length ? seg.slice(1) : [stops[k + 1]])));
  return out;
}
