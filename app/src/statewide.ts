// Statewide view helpers (F34 in the app): the loading screen text, real rail
// and intercity lines drawn for reference, and the county under the camera.
import { km } from "./geo";
import { isRegional, type LngLat, type LoadProgress, type Mode, type RouteSpec } from "./sim/api";

/** A real line from the pipeline's transit.json that sim-core does not run (shell `RefLine`). */
export interface RefLineOut {
  id: string;
  name: string;
  operator: string;
  mode: string; // OSM route type: train, light_rail or tram
  ref: string;
  stops: { name: string; lon: number; lat: number }[];
}

// Line colors: the operator's own where the ref names one, else by mode.
const NAMED: Record<string, string> = {
  red: "#d0202e",
  blue: "#0f6fc6",
  green: "#00a84f",
  orange: "#f28c00",
  purple: "#7f3f98",
  yellow: "#ffcc00",
};
const BY_REF: Record<string, string> = { tre: "#1d4f91", tex: "#b5121b", a: "#78be20" };
const AMTRAK = "#1b3d6d";
const TRAM = "#8c564b";

export function refColor(l: Pick<RefLineOut, "ref" | "operator" | "mode">): string {
  const r = l.ref.toLowerCase();
  if (l.operator === "Amtrak") return AMTRAK;
  return NAMED[r] ?? BY_REF[r] ?? (l.mode === "tram" ? TRAM : "#555");
}

/**
 * Color for a line of today's network from its GTFS name (DATA-9 names carry the agency and often
 * the line color: "DART DART LIGHT RAIL - BLUE LINE", "METRO 700 METRORAIL RED LINE"). Buses gray.
 */
export function existingColor(name: string, regional: boolean): string {
  if (!regional) return "#8a8a8a";
  const n = name.toLowerCase();
  const named = Object.keys(NAMED).find((c) => new RegExp(`\\b${c} line\\b`).test(n));
  if (named) return NAMED[named];
  if (n.includes("silver line")) return "#8a8d91";
  if (n.includes("trinity railway") || /\btre\b/.test(n)) return BY_REF.tre;
  if (n.includes("texrail")) return BY_REF.tex;
  if (n.includes("a-train")) return BY_REF.a;
  if (n.includes("trolley") || n.includes("streetcar")) return TRAM;
  return "#dc241f";
}

/**
 * Reference lines that today's network does not already run: a reference line is dropped when
 * more than half its stops are within 300 m of a stop of a running regional line (an Amtrak
 * route sharing a terminal with TRE stays) (DATA-9 brought
 * DART, METRORail, TRE, TEXRail and the A-train into existing_lines.json; Amtrak and a few
 * streetcars remain reference only).
 */
export function notRunning(reference: RouteSpec[], running: RouteSpec[]): RouteSpec[] {
  const stops = running.filter((r) => isRegional(r.mode)).flatMap((r) => r.stops.map((s) => s.lngLat));
  const near = (p: LngLat) => stops.some((q) => km(p, q) < 0.3);
  return reference.filter((r) => r.stops.filter((s) => near(s.lngLat)).length * 2 <= r.stops.length);
}

/** A reference line as a client route, for the schematic layout and the stops layer. */
export function fromRef(l: RefLineOut): RouteSpec {
  const mode: Mode = l.mode === "train" ? "commuter_rail" : (l.mode as Mode);
  return {
    id: l.id,
    name: l.name,
    mode,
    color: refColor(l),
    stops: l.stops.map((s, i) => ({ id: `${l.id}-s${i}`, name: s.name || `Stop ${i + 1}`, lngLat: [s.lon, s.lat] })),
    headwayMin: 0,
    fare: 0,
  };
}

/** Loading screen text and bar fill (0-1) for a startup progress report. */
export function loadingText(p: LoadProgress): { stage: string; detail: string; fill: number } {
  const fill = Math.min(0.97, p.expectedS > 0 ? p.elapsedS / p.expectedS : 0);
  const left = p.expectedS - p.elapsedS;
  const detail =
    `${Math.round(p.elapsedS)} s` +
    (left > 3 ? `, about ${Math.round(left)} s left` : left > -10 ? ", nearly done" : ", taking longer than the estimate");
  if (p.stage === "reading") return { stage: "Reading the scenario", detail, fill };
  const where = p.statewide ? `all of Texas, ${p.zones.toLocaleString()} census tracts` : `${p.zones.toLocaleString()} zones`;
  const how = p.warm ? "Settling traffic from the last start's roads" : "Finding the first traffic equilibrium";
  return { stage: `${how}: ${where}`, detail, fill };
}

// Texas county names by FIPS for the metros and their neighbors; others show the code.
const COUNTY: Record<number, string> = {
  48021: "Bastrop", 48027: "Bell", 48029: "Bexar", 48039: "Brazoria", 48041: "Brazos", 48055: "Caldwell",
  48061: "Cameron", 48085: "Collin", 48091: "Comal", 48113: "Dallas", 48121: "Denton", 48135: "Ector",
  48139: "Ellis", 48141: "El Paso", 48157: "Fort Bend", 48167: "Galveston", 48187: "Guadalupe", 48201: "Harris",
  48209: "Hays", 48215: "Hidalgo", 48245: "Jefferson", 48251: "Johnson", 48257: "Kaufman", 48303: "Lubbock",
  48309: "McLennan", 48329: "Midland", 48339: "Montgomery", 48355: "Nueces", 48367: "Parker", 48375: "Potter",
  48381: "Randall", 48397: "Rockwall", 48423: "Smith", 48439: "Tarrant", 48441: "Taylor", 48451: "Tom Green",
  48453: "Travis", 48479: "Webb", 48485: "Wichita", 48491: "Williamson",
};

export const countyName = (fips: number) => (COUNTY[fips] ? `${COUNTY[fips]} County` : `County ${fips}`);

/** FIPS of the zone whose centroid is nearest `p` (zones as `[lon, lat, fips]`), or null if none. */
export function countyAt(zones: [number, number, number][], p: LngLat): number | null {
  const k = Math.cos((p[1] * Math.PI) / 180);
  let best: number | null = null;
  let bd = Infinity;
  for (const [lon, lat, fips] of zones) {
    const d = ((lon - p[0]) * k) ** 2 + (lat - p[1]) ** 2;
    if (d < bd) (bd = d), (best = fips);
  }
  return best;
}
