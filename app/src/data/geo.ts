import type { LngLat } from "../sim/api";

export const TEXAS_BOUNDS: [LngLat, LngLat] = [
  [-106.65, 25.84],
  [-93.51, 36.5],
];
export const TRAVIS_CENTER: LngLat = [-97.75, 30.3];

// Coarse outlines used when the basemap cannot load (offline, CI).
export const TEXAS_OUTLINE: LngLat[] = [
  [-106.63, 31.99], [-103.06, 32.0], [-103.04, 36.5], [-100.0, 36.5], [-100.0, 34.56],
  [-99.2, 34.3], [-98.1, 34.13], [-96.5, 33.77], [-95.3, 33.88], [-94.48, 33.64],
  [-94.04, 33.55], [-94.04, 31.99], [-93.6, 31.17], [-93.52, 30.06], [-93.85, 29.7],
  [-94.7, 29.35], [-95.6, 28.75], [-96.6, 28.3], [-97.2, 27.6], [-97.4, 26.8],
  [-97.15, 25.96], [-97.5, 25.88], [-98.3, 26.1], [-99.1, 26.42], [-99.5, 27.5],
  [-100.3, 28.3], [-100.7, 29.1], [-101.4, 29.77], [-102.4, 29.78], [-102.9, 29.2],
  [-103.3, 29.0], [-104.0, 29.32], [-104.7, 30.0], [-105.0, 30.6], [-105.95, 31.36],
  [-106.63, 31.99],
];
export const TRAVIS_OUTLINE: LngLat[] = [
  [-98.17, 30.36], [-97.77, 30.63], [-97.37, 30.42], [-97.37, 30.2],
  [-97.65, 30.07], [-97.92, 30.03], [-98.17, 30.25], [-98.17, 30.36],
];

// CapMetro Red Line, approximate station positions (demo preset).
export const RED_LINE: { name: string; lngLat: LngLat }[] = [
  { name: "Downtown", lngLat: [-97.7393, 30.2643] },
  { name: "Plaza Saltillo", lngLat: [-97.7247, 30.2627] },
  { name: "MLK Jr.", lngLat: [-97.7085, 30.281] },
  { name: "Highland", lngLat: [-97.718, 30.3268] },
  { name: "Crestview", lngLat: [-97.7186, 30.3396] },
  { name: "Kramer", lngLat: [-97.718, 30.3925] },
  { name: "Howard", lngLat: [-97.695, 30.4385] },
  { name: "Lakeline", lngLat: [-97.798, 30.479] },
  { name: "Leander", lngLat: [-97.853, 30.588] },
];
