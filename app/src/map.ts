// Map setup: basemap, static outlines, and empty sources/layers the app fills.
import maplibregl, { type StyleSpecification } from "maplibre-gl";
import { TEXAS_OUTLINE, TRAVIS_CENTER, TRAVIS_OUTLINE } from "./data/geo";

// Free, keyless vector basemap. Override with VITE_MAP_STYLE.
const STYLE_URL = import.meta.env.VITE_MAP_STYLE ?? "https://tiles.openfreemap.org/styles/liberty";

// Zoom bands from the design doc (view width -> zone unit).
/** Zoom at or below which regional lines are pure schematic. */
export const SCHEMATIC_FULL = 10;
/** Zoom at or above which regional lines are pure geography; the morph runs in between. */
export const GEO_FULL = 11.5;
/** Local (bus-like) lines and their stops only render from this zoom (City band). */
export const BUS_MINZOOM = 11;

/** Design-doc zoom band name for a MapLibre zoom level. */
export function zoomBand(z: number): string {
  if (z < 7.5) return "State";
  if (z < 10.5) return "Region";
  if (z < 13.5) return "City";
  return "Neighborhood";
}

const FALLBACK_STYLE: StyleSpecification = {
  version: 8,
  glyphs: "https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf",
  sources: {},
  layers: [{ id: "bg", type: "background", paint: { "background-color": "#eef0ea" } }],
};

async function loadStyle(): Promise<string | StyleSpecification> {
  try {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), 4000);
    const res = await fetch(STYLE_URL, { signal: ctl.signal });
    clearTimeout(t);
    if (!res.ok) throw new Error(String(res.status));
    return (await res.json()) as StyleSpecification;
  } catch {
    console.warn("Basemap unavailable, using offline fallback");
    return FALLBACK_STYLE;
  }
}

const empty = (): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features: [] });
const line = (coords: [number, number][]): GeoJSON.Feature => ({
  type: "Feature",
  properties: {},
  geometry: { type: "LineString", coordinates: coords },
});

/** Create the map, wait for load, and add every app source and layer (empty). */
export async function createMap(container: string): Promise<maplibregl.Map> {
  const map = new maplibregl.Map({
    container,
    style: await loadStyle(),
    center: TRAVIS_CENTER,
    zoom: 11.5,
    minZoom: 4.5,
    maxZoom: 17,
    maxBounds: [
      [-115, 20],
      [-85, 42],
    ],
    attributionControl: { compact: true },
  });
  map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "bottom-right");
  await new Promise<void>((r) => map.once("load", () => r()));

  const src = (id: string, data: GeoJSON.FeatureCollection | GeoJSON.Feature = empty()) =>
    map.addSource(id, { type: "geojson", data });

  // Fades the basemap so the schematic reads cleanly when zoomed out.
  src("wash", {
    type: "Feature",
    properties: {},
    geometry: { type: "Polygon", coordinates: [[[-180, -85], [180, -85], [180, 85], [-180, 85], [-180, -85]]] },
  });
  map.addLayer({
    id: "wash",
    type: "fill",
    source: "wash",
    paint: {
      "fill-color": "#ffffff",
      "fill-opacity": ["interpolate", ["linear"], ["zoom"], SCHEMATIC_FULL, 0.6, GEO_FULL, 0],
    },
  });

  src("outlines", { type: "FeatureCollection", features: [line(TEXAS_OUTLINE), line(TRAVIS_OUTLINE)] });
  map.addLayer({
    id: "outlines",
    type: "line",
    source: "outlines",
    paint: { "line-color": "#8a8f98", "line-width": 1.2, "line-dasharray": [3, 2] },
  });

  // Desire lines (F06): straight zone-to-zone flows under the transit layers.
  src("desire");
  map.addLayer({
    id: "desire",
    type: "line",
    source: "desire",
    layout: { "line-cap": "round" },
    paint: {
      "line-width": ["interpolate", ["linear"], ["sqrt", ["get", "rel"]], 0, 0.5, 1, 9],
      "line-color": ["interpolate", ["linear"], ["get", "transitShare"], 0, "#7a5195", 0.1, "#2a9d8f"],
      "line-opacity": 0.45,
    },
  });

  src("bus-lines");
  map.addLayer({
    id: "bus-lines",
    type: "line",
    source: "bus-lines",
    minzoom: BUS_MINZOOM,
    layout: { "line-cap": "round", "line-join": "round" },
    paint: { "line-color": ["get", "color"], "line-width": 4 },
  });
  src("rail-lines");
  map.addLayer({
    id: "rail-lines",
    type: "line",
    source: "rail-lines",
    layout: { "line-cap": "round", "line-join": "round" },
    paint: { "line-color": ["get", "color"], "line-width": ["interpolate", ["linear"], ["zoom"], 6, 3, 12, 6] },
  });

  src("stops");
  map.addLayer({
    id: "stops",
    type: "circle",
    source: "stops",
    filter: ["any", ["get", "regional"], ["get", "editing"], [">=", ["zoom"], BUS_MINZOOM]],
    paint: {
      // Size by boardings once you are close; uniform tube-style dots when far.
      "circle-radius": [
        "interpolate", ["linear"], ["zoom"],
        SCHEMATIC_FULL, ["case", ["get", "interchange"], 6, 4],
        13, ["+", 4, ["*", 1.2, ["sqrt", ["/", ["get", "boardings"], 20]]]],
      ],
      "circle-color": "#ffffff",
      "circle-stroke-color": ["case", ["get", "editing"], "#111111", ["get", "interchange"], "#111111", ["get", "color"]],
      "circle-stroke-width": ["case", ["get", "interchange"], 3, 2.5],
    },
  });
  map.addLayer({
    id: "stop-labels",
    type: "symbol",
    source: "stops",
    filter: ["any", ["get", "regional"], [">=", ["zoom"], 13]],
    minzoom: 8,
    layout: {
      "text-field": ["get", "label"],
      "text-font": ["Noto Sans Regular"],
      "text-size": 12,
      "text-offset": [0.9, 0],
      "text-anchor": "left",
      "text-optional": true,
    },
    paint: { "text-color": "#1b1d22", "text-halo-color": "#ffffff", "text-halo-width": 1.5 },
  });

  src("draft");
  map.addLayer({
    id: "draft-line",
    type: "line",
    source: "draft",
    paint: { "line-color": "#e4572e", "line-width": 3, "line-dasharray": [2, 1] },
  });
  map.addLayer({
    id: "draft-stops",
    type: "circle",
    source: "draft",
    filter: ["==", ["geometry-type"], "Point"],
    paint: { "circle-radius": 5, "circle-color": "#e4572e" },
  });
  return map;
}

/** Replace the features of one of the app's GeoJSON sources. */
export function setData(map: maplibregl.Map, id: string, features: GeoJSON.Feature[]): void {
  (map.getSource(id) as maplibregl.GeoJSONSource).setData({ type: "FeatureCollection", features });
}
