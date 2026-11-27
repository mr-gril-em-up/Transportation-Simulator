// SimClient backed by crates/sim-core through the Tauri commands in
// app/src-tauri/src/sim.rs. Converts the app's RouteSpec to sim-core's
// TransitLine and LineResult back to RouteResult.
import { invoke } from "@tauri-apps/api/core";
import { isRegional, type RouteResult, type RouteSpec, type SimClient } from "./api";

/** sim-core TransitLine (crates/sim-core/src/schema.rs). Omitted fields take mode defaults. */
interface TransitLine {
  name: string;
  mode: string;
  stops: { lon: number; lat: number; name: string; park_ride: boolean }[];
  headway_min: number;
  fare: number;
}

/** sim-core LineResult (fields used here) plus the client's route id. */
interface LineResultOut {
  route_id: string;
  daily_boardings: number;
  boardings_by_stop: number[]; // aligned with the line's stops
  operating_cost_per_day: number;
  farebox_recovery: number;
}

/** RouteSpec to sim-core TransitLine (field mapping in docs/ARCHITECTURE.md). */
export function toTransitLine(r: RouteSpec): TransitLine {
  return {
    name: r.name,
    mode: r.mode,
    // Regional lines get park-and-ride at every stop: most Texas suburban rail
    // trips start with a drive. Refine per stop when the UI can edit stops.
    stops: r.stops.map((s) => ({ lon: s.lngLat[0], lat: s.lngLat[1], name: s.name, park_ride: isRegional(r.mode) })),
    headway_min: r.headwayMin,
    fare: r.fare,
  };
}

/** sim-core result back to RouteResult; per-stop values keyed by the route's stop ids. */
export function fromLineResult(o: LineResultOut, route: RouteSpec | undefined): RouteResult {
  const boardingsByStop: Record<string, number> = {};
  route?.stops.forEach((s, i) => (boardingsByStop[s.id] = Math.round(o.boardings_by_stop[i] ?? 0)));
  return {
    routeId: o.route_id,
    dailyRiders: Math.round(o.daily_boardings),
    boardingsByStop,
    operatingCostPerDay: o.operating_cost_per_day,
    fareboxRecovery: o.farebox_recovery,
  };
}

/** SimClient that calls sim-core through the desktop shell's Tauri commands. */
export class TauriSim implements SimClient {
  readonly label = "sim-core";
  // Stop ids live only in the client; keep specs to map per-stop results back.
  private routes = new Map<string, RouteSpec>();

  async upsertRoute(route: RouteSpec): Promise<RouteResult[]> {
    this.routes.set(route.id, route);
    const out = await invoke<LineResultOut[]>("upsert_route", { routeId: route.id, line: toTransitLine(route) });
    return this.convert(out);
  }

  async removeRoute(id: string): Promise<RouteResult[]> {
    this.routes.delete(id);
    return this.convert(await invoke<LineResultOut[]>("remove_route", { routeId: id }));
  }

  private convert(out: LineResultOut[]): RouteResult[] {
    return out.map((o) => fromLineResult(o, this.routes.get(o.route_id)));
  }
}
