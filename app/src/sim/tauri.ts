// SimClient backed by crates/sim-core through the Tauri commands in
// app/src-tauri/src/sim.rs. Converts the app's RouteSpec to sim-core's
// TransitLine and LineResult back to RouteResult.
import { invoke } from "@tauri-apps/api/core";
import { isRegional, STATEMENT_MONTHS, type Budget, type Statement, type PendingRoute, type ClockState, type DesireLine, type RoutePreview, type LngLat, type RouteResult, type RouteSpec, type SimClient } from "./api";

/** sim-core TransitLine (crates/sim-core/src/schema.rs). Omitted fields take mode defaults. */
interface TransitLine {
  name: string;
  mode: string;
  stops: { lon: number; lat: number; name: string; park_ride: boolean }[];
  headway_min: number;
  offpeak_headway_min?: number;
  fare: number;
}

/** sim-core LineResult (fields used here) plus the client's route id. */
interface LineResultOut {
  route_id: string;
  daily_boardings: number;
  boardings_by_stop: number[]; // aligned with the line's stops
  operating_cost_per_day: number;
  farebox_recovery: number;
  length_km: number;
  run_min: number;
  vehicles_required: number;
  passenger_km: number;
  segment_loads: [number, number][];
  peak_hour_load: number;
  peak_hour_capacity: number;
}

/** sim-core LinePreview (crates/sim-core/src/model.rs). */
interface LinePreview {
  line: { daily_boardings: number; operating_cost_per_day: number };
  capital_cost: number;
  federal_grant: number;
  opens_day: number; // added by the shell (PreviewOut)
}

/** sim-core MonthStatement (crates/sim-core/src/economy.rs). */
interface MonthStatement {
  month: number;
  fare_revenue: number;
  operating_cost: number;
  operating_assistance: number;
  local_tax: number;
  capital_spent: number;
  federal_capital_grants: number;
  bond_proceeds: number;
  debt_service: number;
  interest: number;
  net: number;
}

/** sim-core Ledger (crates/sim-core/src/economy.rs), fields used here. */
interface Ledger {
  cash: number;
  current: { capital_spent: number; federal_capital_grants: number };
  history: MonthStatement[]; // oldest first
  distressed_months: number;
  bankrupt_month: number | null;
  // Added by the shell (LedgerOut):
  debt: number;
  debt_service_per_month: number;
  bankruptcy_months: number;
  bond_rate: number;
  bond_term_months: number;
}

export const fromPreview = (p: LinePreview): RoutePreview => ({
  dailyRiders: Math.round(p.line.daily_boardings),
  capitalCost: p.capital_cost,
  federalGrant: p.federal_grant,
  operatingCostPerDay: p.line.operating_cost_per_day,
  opensDay: p.opens_day,
});

const fromStatement = (m: MonthStatement): Statement => ({
  month: m.month,
  fares: m.fare_revenue,
  operatingCost: m.operating_cost,
  operatingAssistance: m.operating_assistance,
  localTax: m.local_tax,
  capitalSpent: m.capital_spent,
  federalGrants: m.federal_capital_grants,
  bondProceeds: m.bond_proceeds,
  debtService: m.debt_service,
  interest: m.interest,
  net: m.net,
});

export const fromLedger = (l: Ledger): Budget => ({
  cash: l.cash,
  capitalSpentThisMonth: l.current.capital_spent,
  grantsThisMonth: l.current.federal_capital_grants,
  debt: l.debt,
  debtServicePerMonth: l.debt_service_per_month,
  distressedMonths: l.distressed_months,
  bankruptcyMonths: l.bankruptcy_months,
  bankruptMonth: l.bankrupt_month,
  bondRate: l.bond_rate,
  bondTermMonths: l.bond_term_months,
  statements: l.history.slice(-STATEMENT_MONTHS).reverse().map(fromStatement),
});

/** sim-core Clock (crates/sim-core/src/clock.rs) plus `day`, from the shell's ClockOut. */
interface ClockOut {
  game_seconds: number;
  timelapse: number;
  paused: boolean;
  day: number;
}

export const fromClock = (c: ClockOut): ClockState => ({
  gameSeconds: c.game_seconds,
  timelapse: c.timelapse,
  paused: c.paused,
  day: c.day,
});

/** RouteSpec to sim-core TransitLine (field mapping in docs/ARCHITECTURE.md). */
export function toTransitLine(r: RouteSpec): TransitLine {
  return {
    name: r.name,
    mode: r.mode,
    // Regional lines get park-and-ride at every stop: most Texas suburban rail
    // trips start with a drive. Refine per stop when the UI can edit stops.
    stops: r.stops.map((s) => ({ lon: s.lngLat[0], lat: s.lngLat[1], name: s.name, park_ride: isRegional(r.mode) })),
    headway_min: r.headwayMin,
    ...(r.offpeakHeadwayMin !== undefined && { offpeak_headway_min: r.offpeakHeadwayMin }),
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
    detail: {
      lengthKm: o.length_km,
      runMin: o.run_min,
      vehicles: o.vehicles_required,
      passengerKm: o.passenger_km,
      segmentLoads: o.segment_loads,
      peakHourLoad: o.peak_hour_load,
      peakHourCapacity: o.peak_hour_capacity,
    },
  };
}

/** Panel label for the shell's scenario source: a file path or "synthetic". */
export function sourceLabel(source: string): string {
  if (source === "synthetic") return "sim-core · synthetic region";
  return source.includes("48453") ? "sim-core · Travis data" : `sim-core · ${source.split(/[\\/]/).slice(-2).join("/")}`;
}

/** SimClient that calls sim-core through the desktop shell's Tauri commands. */
export class TauriSim implements SimClient {
  /** "sim-core · Travis data" or "sim-core · synthetic region", once the shell reports its source. */
  label = "sim-core";
  // Stop ids live only in the client; keep specs to map per-stop results back.
  private routes = new Map<string, RouteSpec>();

  async ready(): Promise<void> {
    for (;;) {
      const s = await invoke<{ ready: boolean; error: string | null; source: string }>("sim_status");
      this.label = sourceLabel(s.source);
      if (s.error) throw new Error(s.error);
      if (s.ready) return;
      await new Promise((r) => setTimeout(r, 500));
    }
  }

  async upsertRoute(route: RouteSpec): Promise<RouteResult[]> {
    this.routes.set(route.id, route);
    const out = await invoke<LineResultOut[]>("upsert_route", { routeId: route.id, line: toTransitLine(route) });
    return this.convert(out);
  }

  async removeRoute(id: string): Promise<RouteResult[]> {
    this.routes.delete(id);
    return this.convert(await invoke<LineResultOut[]>("remove_route", { routeId: id }));
  }

  async desireLines(n: number): Promise<DesireLine[]> {
    const out = await invoke<{ from: LngLat; to: LngLat; trips: number; transit_trips: number }[]>("desire_lines", { n });
    return out.map((d) => ({ from: d.from, to: d.to, trips: d.trips, transitTrips: d.transit_trips }));
  }

  async previewRoute(route: RouteSpec): Promise<RoutePreview> {
    return fromPreview(await invoke<LinePreview>("preview_route", { line: toTransitLine(route) }));
  }

  async pendingRoutes(): Promise<PendingRoute[]> {
    const out = await invoke<{ route_id: string; opens_day: number; replaces_open_line: boolean }[]>("pending_routes");
    return out.map((p) => ({ routeId: p.route_id, opensDay: p.opens_day, replacesOpenLine: p.replaces_open_line }));
  }

  async budget(): Promise<Budget> {
    return fromLedger(await invoke<Ledger>("ledger"));
  }

  async issueBond(principal: number): Promise<Budget> {
    return fromLedger(await invoke<Ledger>("issue_bond", { principal }));
  }

  async results(): Promise<RouteResult[]> {
    return this.convert(await invoke<LineResultOut[]>("results"));
  }

  async clock(): Promise<ClockState> {
    return fromClock(await invoke<ClockOut>("clock"));
  }

  async setClock(c: { paused?: boolean; timelapse?: number }): Promise<ClockState> {
    return fromClock(await invoke<ClockOut>("set_clock", { paused: c.paused ?? null, timelapse: c.timelapse ?? null }));
  }

  async skipToNextMonth(): Promise<ClockState> {
    return fromClock(await invoke<ClockOut>("skip_to_next_month"));
  }

  private convert(out: LineResultOut[]): RouteResult[] {
    return out.map((o) => fromLineResult(o, this.routes.get(o.route_id)));
  }
}
