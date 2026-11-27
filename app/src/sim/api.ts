// The only surface the app uses to talk to the simulation.
// Two backends: TauriSim (tauri.ts) calls sim-core through Tauri commands in
// the desktop app; MockSim (mock.ts) runs in a plain browser (`npm run dev`).
// createSim() in index.ts picks one.

/** WGS84 [longitude, latitude] in degrees. */
export type LngLat = [number, number];
// Mirrors sim-core's TransitLine modes (crates/sim-core/src/schema.rs). No hyperloop.
export type Mode =
  | "bus" | "trolleybus" | "brt" | "tram" | "light_rail" | "commuter_rail" | "metro"
  | "high_speed_rail" | "monorail" | "people_mover" | "gondola" | "maglev" | "ferry";

/** Street-running modes stay geographic and appear only at City zoom and closer.
 *  Everything else is "regional" and draws as the tube-style schematic when zoomed out. */
export const LOCAL_MODES: ReadonlySet<Mode> = new Set<Mode>(["bus", "trolleybus", "brt"]);
export const isRegional = (m: Mode) => !LOCAL_MODES.has(m);

/** A stop; `id` is client-unique and keys per-stop results. */
export interface Stop {
  id: string;
  name: string;
  lngLat: LngLat;
}

/** A player-drawn line as the client stores it. */
export interface RouteSpec {
  id: string;
  name: string;
  mode: Mode;
  color: string;
  stops: Stop[];
  headwayMin: number; // minutes between vehicles
  fare: number; // USD per boarding
}

/** Simulation output for one route. */
export interface RouteResult {
  routeId: string;
  dailyRiders: number; // weekday boardings
  boardingsByStop: Record<string, number>;
  /** Economics, when the backend provides them (sim-core does, the mock does not). */
  operatingCostPerDay?: number;
  fareboxRecovery?: number; // fare revenue / operating cost, 0..n
}

/** A zone pair and its daily travel demand (F06 desire lines). */
export interface DesireLine {
  from: LngLat;
  to: LngLat;
  trips: number; // daily person trips, both directions, all modes
  transitTrips: number; // daily transit trips, both directions
}

/** What a line would cost and carry if built now (F16 build cost preview). Nothing is charged. */
export interface RoutePreview {
  dailyRiders: number;
  capitalCost: number; // USD: guideway per km plus vehicles
  federalGrant: number; // USD back from the federal government; 0 if it fails the cost-per-rider gate
  operatingCostPerDay?: number;
}

/** The transit authority's books, as the HUD shows them. */
export interface Budget {
  cash: number; // USD on hand; negative until bonds exist (F26)
  capitalSpentThisMonth: number;
  grantsThisMonth: number; // federal capital grants received this month
}

export interface SimClient {
  /** Shown in the ridership panel so it is obvious which backend produced the numbers. */
  readonly label: string;
  /** Resolves once the backend can answer without waiting (sim-core builds for seconds at startup); rejects if it failed to start. */
  ready(): Promise<void>;
  /** Add or replace a route, re-run demand, and return results for every route. */
  upsertRoute(route: RouteSpec): Promise<RouteResult[]>;
  removeRoute(id: string): Promise<RouteResult[]>;
  /** Top `n` zone pairs by daily trips, largest first. */
  desireLines(n: number): Promise<DesireLine[]>;
  /** Cost and ridership of `route` without building it. */
  previewRoute(route: RouteSpec): Promise<RoutePreview>;
  budget(): Promise<Budget>;
}
