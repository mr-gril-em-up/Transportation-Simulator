// Stop editing for one line at a time: drag a stop to move it, click the map to
// insert a stop where it lengthens the line least, right-click a stop to delete
// it (a line keeps at least two). Every finished edit calls onChange once; drags
// preview locally and only re-run the sim on release.
import type maplibregl from "maplibre-gl";
import type { LngLat, RouteSpec } from "./sim/api";
import { km } from "./sim/mock";

/** Index at which inserting `p` adds the least length to the polyline `pts` (0..pts.length). */
export function bestInsertIndex(pts: LngLat[], p: LngLat): number {
  let best = 0;
  let bestCost = km(p, pts[0]); // prepend
  const end = km(pts[pts.length - 1], p); // append
  if (end < bestCost) [best, bestCost] = [pts.length, end];
  for (let i = 0; i + 1 < pts.length; i++) {
    const cost = km(pts[i], p) + km(p, pts[i + 1]) - km(pts[i], pts[i + 1]);
    if (cost < bestCost) [best, bestCost] = [i + 1, cost];
  }
  return best;
}

let stopSeq = 0;
/** A stop id unique for the session, so per-stop results never collide after edits. */
export const newStopId = (routeId: string) => `${routeId}-e${++stopSeq}`;

/** Edit mode for one line; owns map gestures while active. */
export class StopEditor {
  route: RouteSpec | null = null;
  private dragIdx = -1;

  constructor(
    private map: maplibregl.Map,
    private onPreview: () => void, // re-render without a sim call (during drag)
    private onChange: (route: RouteSpec) => void, // edit finished: re-run sim
    private onStateChange: (route: RouteSpec | null) => void,
  ) {
    map.on("mousedown", "stops", (e) => this.down(e));
    map.on("mousemove", (e) => this.move(e));
    map.on("mouseup", () => this.up());
    map.on("click", (e) => this.click(e));
    map.on("contextmenu", "stops", (e) => this.remove(e));
    window.addEventListener("keydown", (e) => e.key === "Escape" && this.stop());
  }

  get active(): boolean {
    return this.route !== null;
  }

  start(route: RouteSpec): void {
    this.route = route;
    const lons = route.stops.map((s) => s.lngLat[0]);
    const lats = route.stops.map((s) => s.lngLat[1]);
    this.map.fitBounds(
      [
        [Math.min(...lons), Math.min(...lats)],
        [Math.max(...lons), Math.max(...lats)],
      ],
      { padding: 80, maxZoom: 14, duration: 500 },
    );
    this.map.getCanvas().style.cursor = "copy";
    this.onStateChange(route);
  }

  stop(): void {
    if (!this.route) return;
    this.route = null;
    this.dragIdx = -1;
    this.map.dragPan.enable();
    this.map.getCanvas().style.cursor = "";
    this.onStateChange(null);
  }

  /** Stop index under the event if it belongs to the line being edited. */
  private hitIdx(e: maplibregl.MapLayerMouseEvent): number {
    const f = e.features?.find((x) => x.properties?.routeId === this.route?.id);
    return f ? Number(f.properties!.idx) : -1;
  }

  private down(e: maplibregl.MapLayerMouseEvent): void {
    if (!this.route || e.originalEvent.button !== 0) return;
    const idx = this.hitIdx(e);
    if (idx < 0) return;
    e.preventDefault();
    this.dragIdx = idx;
    this.map.dragPan.disable();
  }

  private move(e: maplibregl.MapMouseEvent): void {
    if (!this.route || this.dragIdx < 0) return;
    this.route.stops[this.dragIdx].lngLat = [e.lngLat.lng, e.lngLat.lat];
    this.onPreview();
  }

  private up(): void {
    if (!this.route || this.dragIdx < 0) return;
    this.dragIdx = -1;
    this.map.dragPan.enable();
    this.dragEndedAt = performance.now();
    this.onChange(this.route);
  }
  // A click can follow a drag's mouseup (same element); it must not insert a stop.
  // Not every drag produces one, so use a short time window instead of a flag.
  private dragEndedAt = -Infinity;

  private click(e: maplibregl.MapMouseEvent): void {
    if (!this.route) return;
    if (performance.now() - this.dragEndedAt < 250) return;
    if (this.map.queryRenderedFeatures(e.point, { layers: ["stops"] }).length) return;
    const p: LngLat = [e.lngLat.lng, e.lngLat.lat];
    const i = bestInsertIndex(
      this.route.stops.map((s) => s.lngLat),
      p,
    );
    this.route.stops.splice(i, 0, { id: newStopId(this.route.id), name: `Stop ${this.route.stops.length + 1}`, lngLat: p });
    this.onChange(this.route);
  }

  private remove(e: maplibregl.MapLayerMouseEvent): void {
    if (!this.route) return;
    const idx = this.hitIdx(e);
    if (idx < 0 || this.route.stops.length <= 2) return;
    e.preventDefault();
    e.originalEvent.preventDefault(); // no webview context menu
    this.route.stops.splice(idx, 1);
    this.onChange(this.route);
  }
}
