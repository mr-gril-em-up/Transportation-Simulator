// Stand-in for sim-core: a tiny gravity + binary-logit model over synthetic
// Travis County zones. Deterministic, responds to stop placement, headway and
// fare, and runs in a few ms. Numbers are plausible, not calibrated.
import { km } from "../geo";
import { isRegional, type Budget, type PendingRoute, type ClockState, type DesireLine, type RoutePreview, type LngLat, type RouteResult, type RouteSpec, type SimClient } from "./api";

interface Zone {
  lngLat: LngLat;
  pop: number;
  jobs: number;
}

// Job centers: [lng, lat, jobs]
const CENTERS: [number, number, number][] = [
  [-97.743, 30.268, 140000], // Downtown
  [-97.736, 30.285, 60000], // UT Austin / Capitol
  [-97.725, 30.402, 45000], // Domain
  [-97.67, 30.196, 20000], // Airport
  [-97.705, 30.298, 15000], // Mueller
  [-97.678, 30.508, 30000], // Round Rock
  [-97.82, 30.39, 20000], // Northwest Hills / 360
  [-97.79, 30.23, 15000], // South Lamar / Brodie
];

const DOWNTOWN: LngLat = [-97.743, 30.268];


/** ~2 km grid over Travis County with population decaying from downtown. */
export function buildZones(): Zone[] {
  const zones: Zone[] = [];
  const step = 0.02;
  for (let lng = -98.15; lng <= -97.37; lng += step) {
    for (let lat = 30.03; lat <= 30.62; lat += step) {
      const p: LngLat = [lng, lat];
      const d = km(p, DOWNTOWN);
      const pop = 6000 * Math.exp(-d / 12) + 200;
      let jobs = 1500 * Math.exp(-d / 15);
      for (const [x, y, j] of CENTERS) jobs += j * 0.3 * Math.exp(-km(p, [x, y]) / 1.5);
      zones.push({ lngLat: p, pop, jobs });
    }
  }
  return zones;
}

// The mock only distinguishes local (bus-like) from regional (rail-like) modes.
type Kind = "local" | "regional";
const SPEED_KMH = { local: 18, regional: 45 };
// Rail catchment includes park-and-ride, the norm in Texas suburbs.
const CATCH_KM = { local: 0.8, regional: 2.5 };
// Access + egress minutes: short walks for bus, drive or long walk for rail.
const ACCESS_MIN = { local: 4, regional: 8 };
// Riders arriving by transfer from the rest of the (unmodeled) network.
const NETWORK_FACTOR = { local: 4, regional: 1 };
const CAR_KMH = 40;
const VALUE_OF_TIME = 18 / 60; // $ per minute
const TRIPS_PER_PERSON = 2.5; // daily trips that leave home or return to it
const decay = (d: number) => 1 / Math.max(d, 1) ** 1.2;
// Binary logit: utility per generalized-cost minute, and the car's constant.
// ASC tuned so the Red Line demo (30-min headway, $3.50) lands near 2,000/day, a little above
// APTA's ~1,500 a weekday; sim-core (SIM-10) gives 861 alone, 1,367 in today's network (SIM-12) (docs/MOCK-SIM.md).
const COST_COEF = 0.06;
const ASC_CAR = 2.0;

/** Weekday boardings for one route against the synthetic zones (no line interaction). */
export function simulateRoute(route: RouteSpec, zones: Zone[]): RouteResult {
  const stops = route.stops;
  const kind: Kind = isRegional(route.mode) ? "regional" : "local";
  const r = CATCH_KM[kind];
  // People and jobs within walking reach of each stop (soft catchment).
  // allJobs: every job in the county weighted by distance, the gravity denominator.
  const reach = stops.map((s) => {
    let pop = 0;
    let jobs = 0;
    let allJobs = 0;
    for (const z of zones) {
      const d = km(z.lngLat, s.lngLat);
      const w = Math.exp(-((d / r) ** 2));
      pop += z.pop * w;
      jobs += z.jobs * w;
      allJobs += z.jobs * decay(d);
    }
    return { pop, jobs, allJobs };
  });
  // Cumulative distance along the line.
  const along = [0];
  for (let i = 1; i < stops.length; i++) along.push(along[i - 1] + km(stops[i - 1].lngLat, stops[i].lngLat));

  const boardings: Record<string, number> = Object.fromEntries(stops.map((s) => [s.id, 0]));
  let total = 0;
  for (let i = 0; i < stops.length; i++) {
    for (let j = 0; j < stops.length; j++) {
      if (i === j) continue;
      const rideKm = Math.abs(along[j] - along[i]);
      if (rideKm < 0.8) continue;
      const directKm = km(stops[i].lngLat, stops[j].lngLat);
      // Generalized cost in minutes. Wait and walk weigh double.
      const transit =
        2 * (route.headwayMin / 2) + 2 * ACCESS_MIN[kind] + (rideKm / SPEED_KMH[kind]) * 60 + route.fare / VALUE_OF_TIME;
      const car = 6 + (directKm * 1.3 / CAR_KMH) * 60 + (directKm * 0.25) / VALUE_OF_TIME;
      const share = 1 / (1 + Math.exp(COST_COEF * (transit - car) + ASC_CAR));
      // Trips from i's catchment that go to j's catchment, then the transit share.
      const trips = (reach[i].pop * TRIPS_PER_PERSON * reach[j].jobs * decay(directKm)) / reach[i].allJobs * share * NETWORK_FACTOR[kind];
      boardings[stops[i].id] += trips;
      total += trips;
    }
  }
  for (const k in boardings) boardings[k] = Math.round(boardings[k]);
  return { routeId: route.id, dailyRiders: Math.round(total), boardingsByStop: boardings };
}

// Capital per km of guideway and per vehicle, copied from sim-core's sourced
// rows (crates/sim-core/src/params.rs): bus, Austin light rail ($840M/mi),
// commuter rail. Other modes use their kind's row.
const CAPITAL: Partial<Record<RouteSpec["mode"], [number, number]>> = {
  bus: [0.3e6, 716_400],
  light_rail: [840e6 / 1.609344, 5e6],
  commuter_rail: [20e6, 5e6],
};
const FEDERAL_SHARE = 0.5;
const FEDERAL_MAX_PER_RIDER = 600_000; // capital per daily boarding, the grant gate

/** Capital cost and grant for a route: guideway km plus the fleet a round trip at its headway needs. */
export function mockCapital(route: RouteSpec, dailyRiders: number): { capitalCost: number; federalGrant: number } {
  const kind: Kind = isRegional(route.mode) ? "regional" : "local";
  const [perKm, perVehicle] = CAPITAL[route.mode] ?? CAPITAL[kind === "local" ? "bus" : "commuter_rail"]!;
  let len = 0;
  for (let i = 1; i < route.stops.length; i++) len += km(route.stops[i - 1].lngLat, route.stops[i].lngLat);
  const vehicles = Math.ceil((2 * (len / SPEED_KMH[kind]) * 60) / route.headwayMin) + 1;
  const capitalCost = perKm * len + vehicles * perVehicle;
  const passes = dailyRiders > 0 && capitalCost / dailyRiders <= FEDERAL_MAX_PER_RIDER;
  return { capitalCost, federalGrant: passes ? capitalCost * FEDERAL_SHARE : 0 };
}

/** Browser-only SimClient backed by `simulateRoute` (see docs/MOCK-SIM.md). */
export class MockSim implements SimClient {
  readonly label = "mock model (browser)";

  async ready(): Promise<void> {}
  private zones = buildZones();
  private routes = new Map<string, RouteSpec>();
  // Same rules as sim-core: an edit pays only capital beyond what the line already paid.
  private paid = new Map<string, number>();
  private ledger: Budget = {
    cash: 0,
    capitalSpentThisMonth: 0,
    grantsThisMonth: 0,
    debt: 0,
    debtServicePerMonth: 0,
    distressedMonths: 0,
    bankruptcyMonths: 12,
    bankruptMonth: null,
    bondRate: 0.0543,
    bondTermMonths: 360,
    bondCapacity: Infinity, // the mock has no tax base or borrowing cap
    taxCapacity: 0,
    statements: [], // the mock never settles a month
  };

  async upsertRoute(route: RouteSpec): Promise<RouteResult[]> {
    this.routes.set(route.id, route);
    const res = simulateRoute(route, this.zones);
    const { capitalCost } = mockCapital(route, res.dailyRiders);
    const spend = Math.max(0, capitalCost - (this.paid.get(route.id) ?? 0));
    this.paid.set(route.id, Math.max(capitalCost, this.paid.get(route.id) ?? 0));
    const grant = spend > 0 && capitalCost / Math.max(res.dailyRiders, 1e-9) <= FEDERAL_MAX_PER_RIDER ? spend * FEDERAL_SHARE : 0;
    this.ledger.capitalSpentThisMonth += spend;
    this.ledger.grantsThisMonth += grant;
    this.ledger.cash += grant - spend;
    return this.run();
  }

  async previewRoute(route: RouteSpec): Promise<RoutePreview> {
    const { dailyRiders } = simulateRoute(route, this.zones);
    return { dailyRiders, ...mockCapital(route, dailyRiders) };
  }

  /** The mock builds instantly: nothing is ever under construction. */
  /** The mock always starts on a blank map. */
  async existingRoutes(): Promise<RouteSpec[]> {
    return [];
  }

  async pendingRoutes(): Promise<PendingRoute[]> {
    return [];
  }

  /** Cash and debt only: the mock never settles months, so no payments or bankruptcy. */
  async issueBond(principal: number): Promise<Budget> {
    if (!(principal > 0)) throw new Error(`bond principal ${principal} must be positive`);
    const r = this.ledger.bondRate / 12;
    this.ledger.cash += principal;
    this.ledger.debt += principal;
    this.ledger.debtServicePerMonth += (principal * r) / (1 - (1 + r) ** -this.ledger.bondTermMonths);
    return { ...this.ledger };
  }

  async budget(): Promise<Budget> {
    return { ...this.ledger };
  }

  async results(): Promise<RouteResult[]> {
    return this.run();
  }

  // Same rules as sim-core's Clock: starts day 0 05:00, 60 game s per real s
  // at 1x, 30-day months. Advanced from wall time on each call. The mock has
  // no daily re-run or monthly settlement.
  private clk = { gameSeconds: 5 * 3600, timelapse: 1, paused: false };
  private wall = Date.now();

  async clock(): Promise<ClockState> {
    const now = Date.now();
    if (!this.clk.paused) this.clk.gameSeconds += ((now - this.wall) / 1000) * 60 * this.clk.timelapse;
    this.wall = now;
    return { ...this.clk, day: Math.floor(this.clk.gameSeconds / 86400) };
  }

  async setClock(c: { paused?: boolean; timelapse?: number }): Promise<ClockState> {
    await this.clock(); // bank time at the old speed first
    if (c.paused !== undefined) this.clk.paused = c.paused;
    if (c.timelapse !== undefined && c.timelapse > 0) this.clk.timelapse = c.timelapse;
    return this.clock();
  }

  async skipToNextMonth(): Promise<ClockState> {
    const day = Math.floor((await this.clock()).day / 30 + 1) * 30;
    this.clk.gameSeconds = day * 86400;
    return this.clock();
  }

  async removeRoute(id: string): Promise<RouteResult[]> {
    this.routes.delete(id);
    this.paid.delete(id);
    return this.run();
  }

  /** Gravity flows between the 150 busiest zones; no transit split (transitTrips = 0). */
  async desireLines(n: number): Promise<DesireLine[]> {
    const top = [...this.zones].sort((a, b) => b.pop + b.jobs - (a.pop + a.jobs)).slice(0, 150);
    const out: DesireLine[] = [];
    for (let i = 0; i < top.length; i++)
      for (let j = i + 1; j < top.length; j++) {
        const d = Math.max(km(top[i].lngLat, top[j].lngLat), 1);
        const trips = ((top[i].pop * top[j].jobs + top[j].pop * top[i].jobs) * TRIPS_PER_PERSON) / 7e5 / d ** 1.2;
        out.push({ from: top[i].lngLat, to: top[j].lngLat, trips, transitTrips: 0 });
      }
    return out.sort((a, b) => b.trips - a.trips).slice(0, n);
  }

  private run(): RouteResult[] {
    return [...this.routes.values()].map((r) => simulateRoute(r, this.zones));
  }
}
