// Stand-in for sim-core: a tiny gravity + binary-logit model over synthetic
// Travis County zones. Deterministic, responds to stop placement, headway and
// fare, and runs in a few ms. Numbers are plausible, not calibrated.
import { isRegional, type LngLat, type RouteResult, type RouteSpec, type SimClient } from "./api";

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

export function km(a: LngLat, b: LngLat): number {
  const kx = 111.32 * Math.cos(((a[1] + b[1]) / 2) * (Math.PI / 180));
  return Math.hypot((a[0] - b[0]) * kx, (a[1] - b[1]) * 110.57);
}

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
// ASC tuned so the Red Line demo at 30-min headway lands near its real ~2,500/day.
const COST_COEF = 0.06;
const ASC_CAR = 1.9;

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

export class MockSim implements SimClient {
  private zones = buildZones();
  private routes = new Map<string, RouteSpec>();

  async upsertRoute(route: RouteSpec): Promise<RouteResult[]> {
    this.routes.set(route.id, route);
    return this.run();
  }

  async removeRoute(id: string): Promise<RouteResult[]> {
    this.routes.delete(id);
    return this.run();
  }

  private run(): RouteResult[] {
    return [...this.routes.values()].map((r) => simulateRoute(r, this.zones));
  }
}
