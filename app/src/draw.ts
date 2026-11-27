// Click-to-place line drawing. Each click is a stop. Clicking near an
// existing stop reuses its position so lines can share interchanges.
import type maplibregl from "maplibre-gl";
import type { LngLat, Mode } from "./sim/api";
import { setData } from "./map";

export class LineDrawer {
  private pts: LngLat[] = [];
  private mode: Mode | null = null;

  constructor(
    private map: maplibregl.Map,
    private onFinish: (mode: Mode, pts: LngLat[]) => void,
    private onStateChange: (drawing: boolean) => void,
  ) {
    map.on("click", (e) => this.click(e));
    map.on("dblclick", (e) => {
      if (!this.mode) return;
      e.preventDefault();
      this.finish();
    });
    window.addEventListener("keydown", (e) => {
      if (!this.mode) return;
      if (e.key === "Enter") this.finish();
      if (e.key === "Escape") this.cancel();
    });
  }

  get active(): boolean {
    return this.mode !== null;
  }

  start(mode: Mode): void {
    this.mode = mode;
    this.pts = [];
    this.map.doubleClickZoom.disable();
    this.map.getCanvas().style.cursor = "crosshair";
    this.render();
    this.onStateChange(true);
  }

  cancel(): void {
    this.mode = null;
    this.pts = [];
    this.map.doubleClickZoom.enable();
    this.map.getCanvas().style.cursor = "";
    this.render();
    this.onStateChange(false);
  }

  private click(e: maplibregl.MapMouseEvent): void {
    if (!this.mode) return;
    const hit = this.map.queryRenderedFeatures(
      [
        [e.point.x - 10, e.point.y - 10],
        [e.point.x + 10, e.point.y + 10],
      ],
      { layers: ["stops"] },
    )[0];
    const p: LngLat = hit
      ? ((hit.geometry as GeoJSON.Point).coordinates as LngLat)
      : [e.lngLat.lng, e.lngLat.lat];
    // A dblclick fires two clicks first; ignore the duplicate.
    const last = this.pts[this.pts.length - 1];
    if (last && last[0] === p[0] && last[1] === p[1]) return;
    this.pts.push(p);
    this.render();
  }

  private finish(): void {
    const mode = this.mode!;
    const pts = this.pts;
    this.cancel();
    if (pts.length >= 2) this.onFinish(mode, pts);
  }

  private render(): void {
    const f: GeoJSON.Feature[] = this.pts.map((c) => ({
      type: "Feature",
      properties: {},
      geometry: { type: "Point", coordinates: c },
    }));
    if (this.pts.length > 1)
      f.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: this.pts } });
    setData(this.map, "draft", f);
  }
}
