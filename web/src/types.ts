// Mirrors crates/takserver/src/wire.rs.

export interface TakEvent {
  uid: string;
  type: string;
  how: string;
  time: number;
  start: number;
  stale: number;
  lat: number;
  lon: number;
  hae: number;
  ce: number;
  source: "mesh" | "tcp" | "web";
  callsign?: string;
  team?: string;
  role?: string;
  battery?: number;
  speed?: number;
  course?: number;
  sidc?: string;
  label?: string;
  remarks?: string;
  chat?: Chat;
  deletes?: string[];
}

export interface Chat {
  id: string;
  sender_uid: string;
  sender_callsign: string;
  room: string;
  room_id: string;
  text: string;
  time: number;
}

export type ServerMessage = { kind: "event"; event: TakEvent } | { kind: "error"; message: string };

export type ClientOp =
  | { op: "hello"; uid: string; callsign: string; team: string; role: string }
  | { op: "sa"; lat: number; lon: number; speed?: number; course?: number; gps?: boolean }
  | { op: "chat"; id: string; text: string; to?: { uid: string; callsign: string } }
  | { op: "marker"; uid: string; type: string; name: string; lat: number; lon: number; remarks?: string }
  | { op: "delete"; uid: string };

export const ALL_CHAT_ROOMS = "All Chat Rooms";

/** A conversation: All Chat Rooms, or a direct thread keyed by the other party's uid. */
export interface Room {
  key: string;
  name: string;
}

export const ALL_ROOM: Room = { key: ALL_CHAT_ROOMS, name: ALL_CHAT_ROOMS };

export function roomOf(c: Chat, myUid?: string): Room {
  if (c.room_id === ALL_CHAT_ROOMS) return ALL_ROOM;
  // The sender names a direct room after the recipient, so the far side is whoever isn't us.
  if (c.sender_uid === myUid) return { key: c.room_id, name: c.room };
  return { key: c.sender_uid, name: c.sender_callsign };
}

export interface Identity {
  uid: string;
  callsign: string;
  team: string;
  role: string;
}

// ATAK's team palette.
export const TEAM_COLORS: Record<string, string> = {
  White: "#ffffff",
  Yellow: "#ffff00",
  Orange: "#ff7700",
  Magenta: "#ff00ff",
  Red: "#ff0000",
  Maroon: "#7f0000",
  Purple: "#7f007f",
  "Dark Blue": "#00007f",
  Blue: "#0000ff",
  Cyan: "#00ffff",
  Teal: "#007f7f",
  Green: "#00ff00",
  "Dark Green": "#007f00",
  Brown: "#a0714f",
};

export const TEAMS = Object.keys(TEAM_COLORS);
export const ROLES = ["Team Member", "Team Lead", "HQ", "Sniper", "Medic", "Forward Observer", "RTO", "K9"];

/// Kept on the map, faded, this long after going stale (matches the server's grace period).
export const STALE_GRACE_MS = 10 * 60 * 1000;
