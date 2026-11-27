import type { LngLat } from "../sim/api";

/** Texas bounding box, [southwest, northeast], WGS84 degrees. */
export const TEXAS_BOUNDS: [LngLat, LngLat] = [
  [-106.65, 25.84],
  [-93.51, 36.5],
];
/** Default camera center (Travis County test bench). */
export const TRAVIS_CENTER: LngLat = [-97.75, 30.3];

// Coarse outlines used when the basemap cannot load (offline, CI).
/** Coarse Texas border (~35 points), hand-simplified. */
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
/** Coarse Travis County border (8 points), hand-simplified. */
export const TRAVIS_OUTLINE: LngLat[] = [
  [-98.17, 30.36], [-97.77, 30.63], [-97.37, 30.42], [-97.37, 30.2],
  [-97.65, 30.07], [-97.92, 30.03], [-98.17, 30.25], [-98.17, 30.36],
];

// CapMetro Red Line (route 550), the 10 stations of OSM relation 90283 as
// published in the data pipeline's Travis build (transit.json, data-48453-2023),
// northbound order. Coordinates rounded to about 1 m.
// (c) OpenStreetMap contributors, ODbL.
export const RED_LINE: { name: string; lngLat: LngLat }[] = [
  { name: "Downtown", lngLat: [-97.73810, 30.26463] },
  { name: "Plaza Saltillo", lngLat: [-97.72755, 30.26223] },
  { name: "MLK Jr.", lngLat: [-97.70899, 30.27996] },
  { name: "Highland", lngLat: [-97.71619, 30.32877] },
  { name: "Crestview", lngLat: [-97.71968, 30.33844] },
  { name: "McKalla", lngLat: [-97.71759, 30.38797] },
  { name: "Kramer", lngLat: [-97.71638, 30.39279] },
  { name: "Howard", lngLat: [-97.70176, 30.44017] },
  { name: "Lakeline", lngLat: [-97.78668, 30.48117] },
  { name: "Leander", lngLat: [-97.85573, 30.58659] },
];
