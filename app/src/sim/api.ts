// The only surface the app uses to talk to the simulation.
// Two backends: TauriSim (tauri.ts) calls sim-core through Tauri commands in
// the desktop app; MockSim (mock.ts) runs in a plain browser (`npm run dev`).
// createSim() in index.ts picks one.
import type { ClockState } from "../clock";
export type { ClockState };

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
  headwayMin: number; // minutes between vehicles in the peaks (work trips see this)
  /** Minutes between vehicles midday and evening (non-work trips); absent = same as the peak. */
  offpeakHeadwayMin?: number;
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
  /** Game day (sim-core `Clock::day`) it would open if built now; absent means at once. */
  opensDay?: number;
}

/** A route under construction (F24). Until `opensDay` it carries no riders, or,
 *  for a rebuild (`replacesOpenLine`), its previous version keeps running. */
export interface PendingRoute {
  routeId: string;
  opensDay: number; // game day, sim-core numbering (day 0 = "Day 1")
  replacesOpenLine: boolean;
}

/** The transit authority's books, as the HUD shows them. */
export interface Budget {
  cash: number; // USD on hand; can go negative (12 distressed months in a row is bankruptcy)
  capitalSpentThisMonth: number;
  grantsThisMonth: number; // federal capital grants received this month
  debt: number; // principal still owed on bonds
  debtServicePerMonth: number; // monthly payments on outstanding bonds
  distressedMonths: number; // months in a row with cash < 0 or debt service > operating surplus
  bankruptcyMonths: number; // distressed months that end the game
  bankruptMonth: number | null; // game month the authority went bankrupt
  bondRate: number; // annual rate a new bond gets (0.0543 = 5.43%)
  bondTermMonths: number;
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
  /** Borrow `principal` USD as a bond (cash now, monthly payments from next month). Rejects once bankrupt. */
  issueBond(principal: number): Promise<Budget>;
  /** Routes under construction and their opening days. */
  pendingRoutes(): Promise<PendingRoute[]>;
  /** Results for every route without changing anything (re-read after the day re-runs). */
  results(): Promise<RouteResult[]>;
  /** The simulation's clock (F21): the client polls it and interpolates in between. */
  clock(): Promise<ClockState>;
  /** Pause/resume and/or set time-lapse; omitted fields stay as they are. */
  setClock(c: { paused?: boolean; timelapse?: number }): Promise<ClockState>;
  /** Jump to the first day of next month; settles the month's budget. */
  skipToNextMonth(): Promise<ClockState>;
}
