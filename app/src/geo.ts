import type { LngLat } from "./sim/api";

/** Equirectangular distance in km; accurate to <1% at county scale. */
export function km(a: LngLat, b: LngLat): number {
  const kx = 111.32 * Math.cos(((a[1] + b[1]) / 2) * (Math.PI / 180));
  return Math.hypot((a[0] - b[0]) * kx, (a[1] - b[1]) * 110.57);
}
