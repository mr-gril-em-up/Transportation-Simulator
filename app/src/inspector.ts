// Line inspector (ROADMAP-APP M1/M2): per-stop boardings, segment load
// profile and peak load against capacity for one line, as an HTML string for
// the ridership panel. Pure: no DOM or sim access, so it is unit-tested.

import type { LineDetail } from "./sim/api";
import { escapeHtml } from "./format";

const W = 250; // chart width, px (fits the 260 px panel)
const H = 56; // chart height, px

/** One stop as the inspector needs it: name and daily boardings. */
export interface InspectStop {
  name: string;
  boardings: number;
}

/** Peak-hour load on the busiest segment as a share of capacity (0..n). */
export const peakUse = (d: LineDetail): number => (d.peakHourCapacity > 0 ? d.peakHourLoad / d.peakHourCapacity : 0);

/**
 * Inspector HTML for a line. `stops` are in route order; `d.segmentLoads[k]`
 * is the daily load between stop k and k+1, [forward, reverse].
 */
export function inspectorHtml(stops: InspectStop[], d: LineDetail): string {
  const use = peakUse(d);
  const stats =
    `<div>${d.lengthKm.toFixed(1)} km · ${Math.round(d.runMin)} min end to end · ${d.vehicles} vehicle${d.vehicles === 1 ? "" : "s"}` +
    ` · ${Math.round(d.passengerKm).toLocaleString()} passenger-km/day</div>` +
    `<div class="${use > 1 ? "warn" : ""}">Peak hour on the busiest segment: ${Math.round(d.peakHourLoad).toLocaleString()} riders` +
    ` for ${Math.round(d.peakHourCapacity).toLocaleString()} places (${Math.round(use * 100)}%)` +
    (use > 1 ? ". Over capacity: run it more often." : "") +
    `</div>`;
  return (
    `<div class="inspector">${stats}` +
    `<div class="chart-title">Boardings by stop</div>${boardingsSvg(stops)}` +
    `<div class="chart-title">Daily load by segment: outbound solid, inbound dashed</div>${loadSvg(stops, d.segmentLoads)}` +
    `<div class="ends"><span>${escapeHtml(stops[0]?.name ?? "")}</span><span>${escapeHtml(stops.at(-1)?.name ?? "")}</span></div></div>`
  );
}

/** x of stop i of n, inset so end bars are not clipped. */
const xAt = (i: number, n: number) => (n <= 1 ? W / 2 : 6 + (i * (W - 12)) / (n - 1));

function boardingsSvg(stops: InspectStop[]): string {
  const max = Math.max(1, ...stops.map((s) => s.boardings));
  const bw = Math.max(2, Math.min(14, (W - 12) / Math.max(1, stops.length) - 2));
  const bars = stops
    .map((s, i) => {
      const h = (s.boardings / max) * (H - 4);
      return `<rect x="${(xAt(i, stops.length) - bw / 2).toFixed(1)}" y="${(H - h).toFixed(1)}" width="${bw.toFixed(1)}" height="${h.toFixed(1)}"><title>${escapeHtml(s.name)}: ${Math.round(s.boardings).toLocaleString()}</title></rect>`;
    })
    .join("");
  return `<svg class="chart bars" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}">${bars}</svg>`;
}

function loadSvg(stops: InspectStop[], loads: [number, number][]): string {
  const n = stops.length;
  const max = Math.max(1, ...loads.flat());
  const y = (v: number) => (H - 2 - (v / max) * (H - 6)).toFixed(1);
  // A step per segment: flat from stop k to stop k+1 at that segment's load.
  const path = (dir: 0 | 1) =>
    loads.map((l, k) => `${k ? "L" : "M"}${xAt(k, n).toFixed(1)},${y(l[dir])} L${xAt(k + 1, n).toFixed(1)},${y(l[dir])}`).join(" ");
  const tips = loads
    .map(
      (l, k) =>
        `<rect x="${xAt(k, n).toFixed(1)}" y="0" width="${(xAt(k + 1, n) - xAt(k, n)).toFixed(1)}" height="${H}" fill="transparent"><title>${escapeHtml(stops[k]?.name ?? "")} to ${escapeHtml(stops[k + 1]?.name ?? "")}: ${Math.round(l[0]).toLocaleString()} out, ${Math.round(l[1]).toLocaleString()} in</title></rect>`,
    )
    .join("");
  return `<svg class="chart load" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}"><path class="fwd" d="${path(0)}"/><path class="rev" d="${path(1)}"/>${tips}</svg>`;
}
