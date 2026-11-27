// The only surface the app uses to talk to the simulation.
// Today it is backed by MockSim (mock.ts). When crates/sim-core lands, a
// TauriSim implements this same interface by calling Tauri commands that wrap
// sim-core, and createSim() in index.ts switches to it. Nothing else changes.

export type LngLat = [number, number];
// Mirrors sim-core's TransitLine modes (crates/sim-core/src/schema.rs). No hyperloop.
export type Mode =
  | "bus" | "trolleybus" | "brt" | "tram" | "light_rail" | "commuter_rail" | "metro"
  | "high_speed_rail" | "monorail" | "people_mover" | "gondola" | "maglev" | "ferry";

/** Street-running modes stay geographic and appear only at City zoom and closer.
 *  Everything else is "regional" and draws as the tube-style schematic when zoomed out. */
export const LOCAL_MODES: ReadonlySet<Mode> = new Set<Mode>(["bus", "trolleybus", "brt"]);
export const isRegional = (m: Mode) => !LOCAL_MODES.has(m);

export interface Stop {
  id: string;
  name: string;
  lngLat: LngLat;
}

export interface RouteSpec {
  id: string;
  name: string;
  mode: Mode;
  color: string;
  stops: Stop[];
  headwayMin: number; // minutes between vehicles
  fare: number; // USD per boarding
}

export interface RouteResult {
  routeId: string;
  dailyRiders: number; // weekday boardings
  boardingsByStop: Record<string, number>;
}

export interface SimClient {
  /** Add or replace a route, re-run demand, and return results for every route. */
  upsertRoute(route: RouteSpec): Promise<RouteResult[]>;
  removeRoute(id: string): Promise<RouteResult[]>;
}
