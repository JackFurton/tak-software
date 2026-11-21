import ms from "milsymbol";
import type { Map as MapLibre } from "maplibre-gl";
import { TEAM_COLORS, type TakEvent } from "./types";

const UNKNOWN_SIDC = "SUGP-----------";

/**
 * Image name for an event. Devices reporting a team draw as ATAK does, a team-colored dot with
 * a heading chevron; everything else (markers, unaffiliated units) as its 2525 symbol.
 */
export function iconFor(ev: TakEvent): string {
  if (ev.team && TEAM_COLORS[ev.team]) return `team:${ev.team}`;
  return `sidc:${ev.sidc ?? UNKNOWN_SIDC}`;
}

/** Hook for maplibre's `styleimagemissing`: renders icons the first time a layer asks for one. */
export function addMissingImage(map: MapLibre, id: string) {
  const ratio = window.devicePixelRatio || 1;
  let canvas: HTMLCanvasElement;
  if (id.startsWith("sidc:")) {
    canvas = new ms.Symbol(id.slice(5), { size: 26 * ratio }).asCanvas();
  } else if (id.startsWith("team:")) {
    canvas = teamDot(TEAM_COLORS[id.slice(5)] ?? "#ffffff", ratio);
  } else {
    return;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx || map.hasImage(id)) return;
  map.addImage(id, ctx.getImageData(0, 0, canvas.width, canvas.height), { pixelRatio: ratio });
}

function teamDot(color: string, ratio: number): HTMLCanvasElement {
  const size = 40 * ratio;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(ratio, ratio);
  const mid = 20;

  ctx.beginPath();
  ctx.moveTo(mid, 1);
  ctx.lineTo(mid + 7, 11);
  ctx.lineTo(mid - 7, 11);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.strokeStyle = "#000";
  ctx.lineWidth = 1.5;
  ctx.fill();
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(mid, mid, 9, 0, Math.PI * 2);
  ctx.fill();
  ctx.lineWidth = 2;
  ctx.stroke();
  return canvas;
}
