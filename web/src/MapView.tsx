import { useEffect, useRef } from "react";
import {
  LngLatBounds,
  Map as MapLibre,
  NavigationControl,
  ScaleControl,
  setWorkerUrl,
  type GeoJSONSource,
  type StyleSpecification,
} from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { addMissingImage, iconFor } from "./symbols";
import type { TakEvent } from "./types";

// MapLibre looks for its worker next to its own module, which doesn't exist once bundled.
setWorkerUrl(workerUrl);

export type Basemap = "streets" | "satellite";

const STYLE: StyleSpecification = {
  version: 8,
  glyphs: "https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf",
  sources: {
    streets: {
      type: "raster",
      tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
      tileSize: 256,
      maxzoom: 19,
      attribution: '© <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    },
    satellite: {
      type: "raster",
      tiles: ["https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"],
      tileSize: 256,
      maxzoom: 19,
      attribution: "Imagery © Esri, Maxar, Earthstar Geographics",
    },
    units: { type: "geojson", data: { type: "FeatureCollection", features: [] } },
  },
  layers: [
    { id: "streets", type: "raster", source: "streets" },
    { id: "satellite", type: "raster", source: "satellite", layout: { visibility: "none" } },
    {
      id: "selected",
      type: "circle",
      source: "units",
      filter: ["==", ["get", "selected"], true],
      paint: { "circle-radius": 22, "circle-color": "rgba(255,255,255,0.15)", "circle-stroke-color": "#fff", "circle-stroke-width": 2 },
    },
    {
      id: "me",
      type: "circle",
      source: "units",
      filter: ["==", ["get", "me"], true],
      paint: { "circle-radius": 16, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": "#4ade80", "circle-stroke-width": 3 },
    },
    {
      id: "units",
      type: "symbol",
      source: "units",
      layout: {
        "icon-image": ["get", "icon"],
        "icon-rotate": ["get", "rotate"],
        "icon-rotation-alignment": "map",
        "icon-allow-overlap": true,
        "icon-ignore-placement": true,
        "text-field": ["get", "callsign"],
        "text-font": ["Noto Sans Bold"],
        "text-size": 12,
        "text-anchor": "top",
        "text-offset": [0, 1.5],
        "text-optional": true,
      },
      paint: {
        "icon-opacity": ["case", ["get", "stale"], 0.35, 1],
        "text-opacity": ["case", ["get", "stale"], 0.5, 1],
        "text-color": "#fff",
        "text-halo-color": "#000",
        "text-halo-width": 1.5,
      },
    },
  ],
};

interface Props {
  units: TakEvent[];
  now: number;
  myUid: string | null;
  selected: string | null;
  basemap: Basemap;
  crosshair: boolean;
  flyTo: { lat: number; lon: number; seq: number } | null;
  onSelect: (uid: string | null) => void;
  onMapClick: (lat: number, lon: number) => boolean;
}

export function MapView({ units, now, myUid, selected, basemap, crosshair, flyTo, onSelect, onMapClick }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibre | null>(null);
  const loaded = useRef(false);
  // Data that arrives before the style finishes loading is applied from the load handler.
  const applyData = useRef<() => void>(() => {});
  const fitted = useRef(false);
  const handlers = useRef({ onSelect, onMapClick });
  handlers.current = { onSelect, onMapClick };

  useEffect(() => {
    const m = new MapLibre({
      container: container.current!,
      style: STYLE,
      center: [-77.0353, 38.8895],
      zoom: 13,
      attributionControl: { compact: true },
    });
    m.addControl(new NavigationControl({ visualizePitch: false }), "bottom-right");
    m.addControl(new ScaleControl({ unit: "metric" }), "bottom-right");
    m.on("styleimagemissing", (e) => addMissingImage(m, e.id));
    m.on("load", () => {
      loaded.current = true;
      applyData.current();
    });
    m.on("click", (e) => {
      // In placement modes a click means "here", even on top of another unit.
      if (handlers.current.onMapClick(e.lngLat.lat, e.lngLat.lng)) return;
      const hit = m.queryRenderedFeatures(e.point, { layers: ["units"] })[0];
      handlers.current.onSelect(hit ? String(hit.properties.uid) : null);
    });
    m.on("mouseenter", "units", () => (m.getCanvas().style.cursor = "pointer"));
    m.on("mouseleave", "units", () => (m.getCanvas().style.cursor = ""));
    map.current = m;
    return () => m.remove();
  }, []);

  useEffect(() => {
    const m = map.current;
    if (!m) return;
    const apply = () => {
      const source = m.getSource("units") as GeoJSONSource | undefined;
      source?.setData({
        type: "FeatureCollection",
        features: units.map((u) => ({
          type: "Feature",
          geometry: { type: "Point", coordinates: [u.lon, u.lat] },
          properties: {
            uid: u.uid,
            callsign: u.callsign ?? u.uid,
            icon: iconFor(u),
            rotate: u.team && u.course !== undefined ? u.course : 0,
            stale: now >= u.stale,
            me: u.uid === myUid,
            selected: u.uid === selected,
          },
        })),
      });
      if (!fitted.current && units.length > 0) {
        fitted.current = true;
        const bounds = new LngLatBounds();
        units.forEach((u) => bounds.extend([u.lon, u.lat]));
        m.fitBounds(bounds, { padding: 80, maxZoom: 15, duration: 0 });
      }
    };
    applyData.current = apply;
    if (loaded.current) apply();
  }, [units, now, myUid, selected]);

  useEffect(() => {
    const m = map.current;
    if (!m) return;
    const show = () => {
      m.setLayoutProperty("streets", "visibility", basemap === "streets" ? "visible" : "none");
      m.setLayoutProperty("satellite", "visibility", basemap === "satellite" ? "visible" : "none");
    };
    if (loaded.current) show();
    else m.once("load", show);
  }, [basemap]);

  useEffect(() => {
    if (flyTo) map.current?.flyTo({ center: [flyTo.lon, flyTo.lat], zoom: Math.max(map.current.getZoom(), 15) });
  }, [flyTo]);

  useEffect(() => {
    if (map.current) map.current.getCanvas().style.cursor = crosshair ? "crosshair" : "";
  }, [crosshair]);

  return <div ref={container} className="map" />;
}
