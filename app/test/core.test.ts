import { describe, expect, it } from "vitest";
import { GameClock } from "../src/clock";
import { RED_LINE } from "../src/data/geo";
import { elbow, layoutSchematic } from "../src/schematic";
import type { RouteSpec } from "../src/sim/api";
import { MockSim } from "../src/sim/mock";

const redLine = (headwayMin = 20): RouteSpec => ({
  id: "red",
  name: "Red",
  mode: "commuter_rail",
  color: "#f00",
  headwayMin,
  fare: 1.25,
  stops: RED_LINE.map((s, i) => ({ id: `s${i}`, name: s.name, lngLat: s.lngLat })),
});

describe("clock", () => {
  it("runs 1 game minute per real second at 1x and scales with time-lapse only", () => {
    const c = new GameClock();
    const start = c.minutes;
    c.tick(1);
    expect(c.minutes - start).toBe(1);
    c.cycleLapse(); // 2x
    c.cycleLapse(); // 10x
    c.tick(1);
    expect(c.minutes - start).toBe(11);
    c.paused = true;
    c.tick(5);
    expect(c.minutes - start).toBe(11);
  });
});

describe("schematic", () => {
  const octilinear = (a: number[], b: number[]) => {
    const dx = Math.abs(b[0] - a[0]);
    const dy = Math.abs(b[1] - a[1]);
    return dx < 1e-9 || dy < 1e-9 || Math.abs(dx - dy) < 1e-9;
  };

  it("elbow connects any two points with 45/90 degree legs", () => {
    const a: [number, number] = [0, 0];
    const b: [number, number] = [3, 1];
    const pts = [a, ...elbow(a, b), b];
    for (let i = 1; i < pts.length; i++) expect(octilinear(pts[i - 1], pts[i])).toBe(true);
  });

  it("lays out the Red Line with one point per stop and keeps it near its real location", () => {
    const [l] = layoutSchematic([redLine()], 300);
    expect(l.stops).toHaveLength(RED_LINE.length);
    expect(l.geo).toHaveLength(l.schem.length);
    const cx = l.stops.reduce((a, s) => a + s.schem[0], 0) / l.stops.length;
    expect(cx).toBeGreaterThan(-98);
    expect(cx).toBeLessThan(-97.5);
  });

  it("merges shared stops into one interchange", () => {
    const a = redLine();
    const b: RouteSpec = {
      ...a,
      id: "blue",
      stops: [
        { id: "b0", name: "East", lngLat: [-97.6, 30.3] },
        { id: "b1", name: "Highland", lngLat: RED_LINE[3].lngLat },
        { id: "b2", name: "West", lngLat: [-97.85, 30.33] },
      ],
    };
    const out = layoutSchematic([a, b], 300);
    const hl = out.flatMap((l) => l.stops).filter((s) => s.name === "Highland");
    expect(hl.every((s) => s.interchange)).toBe(true);
    expect(hl[0].schem).toEqual(hl[1].schem);
  });
});

describe("mock sim", () => {
  it("returns plausible Red Line ridership that rises with frequency", async () => {
    const sim = new MockSim();
    const slow = (await sim.upsertRoute(redLine(30)))[0].dailyRiders;
    const fast = (await sim.upsertRoute(redLine(10)))[0].dailyRiders;
    expect(slow).toBeGreaterThan(500);
    expect(slow).toBeLessThan(20000);
    expect(fast).toBeGreaterThan(slow);
  });
});
