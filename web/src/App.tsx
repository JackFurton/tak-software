import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { MapView, type Basemap } from "./MapView";
import { ChatPanel, ContactsPanel, DetailCard, MarkerDialog, SettingsDialog, isMarker, type MarkerDraft } from "./panels";
import { randomId } from "./ids";
import { useTakSocket } from "./socket";
import {
  ALL_ROOM,
  STALE_GRACE_MS,
  TEAM_COLORS,
  roomOf,
  type Chat,
  type Identity,
  type Room,
  type ServerMessage,
  type TakEvent,
} from "./types";

interface State {
  units: Record<string, TakEvent>;
  chats: Chat[];
}

type Action = { type: "event"; event: TakEvent } | { type: "prune"; now: number };

function reducer(state: State, action: Action): State {
  if (action.type === "prune") {
    const units = Object.fromEntries(
      Object.entries(state.units).filter(([, u]) => u.stale + STALE_GRACE_MS > action.now),
    );
    return Object.keys(units).length === Object.keys(state.units).length ? state : { ...state, units };
  }
  const ev = action.event;
  if (ev.deletes?.length) {
    const units = { ...state.units };
    ev.deletes.forEach((uid) => delete units[uid]);
    return { ...state, units };
  }
  if (ev.chat) {
    if (state.chats.some((c) => c.id === ev.chat!.id)) return state;
    const chats = [...state.chats, ev.chat].sort((a, b) => a.time - b.time).slice(-500);
    return { ...state, chats };
  }
  if (!ev.type.startsWith("a-")) return state;
  const prev = state.units[ev.uid];
  if (prev && prev.time > ev.time) return state;
  return { ...state, units: { ...state.units, [ev.uid]: ev } };
}

function loadIdentity(): Identity | null {
  try {
    const raw = localStorage.getItem("tak.identity");
    return raw ? (JSON.parse(raw) as Identity) : null;
  } catch {
    return null;
  }
}

function saveIdentity(id: Identity) {
  try {
    localStorage.setItem("tak.identity", JSON.stringify(id));
  } catch {
    // Private browsing: identity lasts for this tab only.
  }
}

type Mode = "none" | "place-me" | "marker";
type Panel = "contacts" | "chat" | null;
interface MyPosition {
  lat: number;
  lon: number;
  gps: boolean;
  speed?: number;
  course?: number;
}

const SA_INTERVAL_MS = 5000;

export function App() {
  const [identity, setIdentity] = useState<Identity | null>(loadIdentity);
  const [editingIdentity, setEditingIdentity] = useState(identity === null);
  const [state, dispatch] = useReducer(reducer, { units: {}, chats: [] });
  const [now, setNow] = useState(Date.now());
  const [mode, setMode] = useState<Mode>("none");
  const [panel, setPanel] = useState<Panel>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [basemap, setBasemap] = useState<Basemap>("streets");
  const [myPos, setMyPos] = useState<MyPosition | null>(null);
  const [gpsOn, setGpsOn] = useState(false);
  const [draft, setDraft] = useState<MarkerDraft | null>(null);
  const [flyTo, setFlyTo] = useState<{ lat: number; lon: number; seq: number } | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [seenChats, setSeenChats] = useState(0);
  const [room, setRoom] = useState<Room>(ALL_ROOM);

  const onMessage = useCallback((msg: ServerMessage) => {
    if (msg.kind === "event") dispatch({ type: "event", event: msg.event });
    else setToast(msg.message);
  }, []);
  const { status, send } = useTakSocket(identity, onMessage);

  useEffect(() => {
    const t = window.setInterval(() => {
      const n = Date.now();
      setNow(n);
      dispatch({ type: "prune", now: n });
    }, 2000);
    return () => window.clearInterval(t);
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(t);
  }, [toast]);

  // Like a phone, report our own position on a timer whenever we have one.
  const posRef = useRef(myPos);
  posRef.current = myPos;
  useEffect(() => {
    if (!myPos || !identity || status !== "open") return;
    const report = () => {
      const p = posRef.current;
      if (p) send({ op: "sa", lat: p.lat, lon: p.lon, gps: p.gps, speed: p.speed, course: p.course });
    };
    report();
    const t = window.setInterval(report, SA_INTERVAL_MS);
    return () => window.clearInterval(t);
  }, [myPos !== null, myPos?.lat, myPos?.lon, identity, status, send]);

  useEffect(() => {
    if (!gpsOn) return;
    const watch = navigator.geolocation.watchPosition(
      (p) =>
        setMyPos({
          lat: p.coords.latitude,
          lon: p.coords.longitude,
          gps: true,
          speed: p.coords.speed ?? undefined,
          course: p.coords.heading ?? undefined,
        }),
      (err) => {
        setToast(`Location unavailable: ${err.message}`);
        setGpsOn(false);
      },
      { enableHighAccuracy: true },
    );
    return () => navigator.geolocation.clearWatch(watch);
  }, [gpsOn]);

  const units = useMemo(() => Object.values(state.units), [state.units]);
  const selectedUnit = selected ? state.units[selected] : undefined;
  const unread = panel === "chat" ? 0 : Math.max(0, state.chats.length - seenChats);
  const rooms = useMemo(() => {
    const byKey = new Map<string, Room>([[ALL_ROOM.key, ALL_ROOM]]);
    for (const c of state.chats) {
      const r = roomOf(c, identity?.uid);
      byKey.set(r.key, r);
    }
    byKey.set(room.key, byKey.get(room.key) ?? room);
    return [...byKey.values()];
  }, [state.chats, identity?.uid, room]);

  const sendChat = (text: string) => {
    const id = randomId();
    if (room.key === ALL_ROOM.key) return send({ op: "chat", id, text });
    // Address by the contact's current callsign, which is what servers route on.
    const callsign = state.units[room.key]?.callsign ?? room.name;
    send({ op: "chat", id, text, to: { uid: room.key, callsign } });
  };

  useEffect(() => {
    if (panel === "chat") setSeenChats(state.chats.length);
  }, [panel, state.chats.length]);

  const onMapClick = (lat: number, lon: number) => {
    if (mode === "place-me") {
      setGpsOn(false);
      setMyPos({ lat, lon, gps: false });
      setMode("none");
      return true;
    }
    if (mode === "marker") {
      setDraft({ lat, lon });
      setMode("none");
      return true;
    }
    return false;
  };

  const focus = (u: TakEvent) => {
    setSelected(u.uid);
    setFlyTo({ lat: u.lat, lon: u.lon, seq: Date.now() });
  };

  return (
    <div className="app">
      <MapView
        units={units}
        now={now}
        myUid={identity?.uid ?? null}
        selected={selected}
        basemap={basemap}
        crosshair={mode !== "none"}
        flyTo={flyTo}
        onSelect={setSelected}
        onMapClick={onMapClick}
      />

      <header className="toolbar">
        <button className="identity" onClick={() => setEditingIdentity(true)} title="Edit callsign and team">
          <span className="team-dot" style={{ background: TEAM_COLORS[identity?.team ?? "White"] }} />
          {identity?.callsign ?? "Set callsign"}
          <span className={`status status-${status}`} title={`Server: ${status}`} />
        </button>
        <div className="tools">
          <button className={mode === "place-me" ? "active" : ""} onClick={() => setMode(mode === "place-me" ? "none" : "place-me")}>
            {myPos ? "Move me" : "Place me"}
          </button>
          <button
            className={gpsOn ? "active" : ""}
            disabled={!("geolocation" in navigator) || !window.isSecureContext}
            title={window.isSecureContext ? "Report this device's location" : "Location needs https or localhost"}
            onClick={() => setGpsOn(!gpsOn)}
          >
            GPS
          </button>
          <button className={mode === "marker" ? "active" : ""} onClick={() => setMode(mode === "marker" ? "none" : "marker")}>
            Drop marker
          </button>
          <button onClick={() => setBasemap(basemap === "streets" ? "satellite" : "streets")}>
            {basemap === "streets" ? "Satellite" : "Streets"}
          </button>
          <button className={panel === "contacts" ? "active" : ""} onClick={() => setPanel(panel === "contacts" ? null : "contacts")}>
            Contacts <span className="count">{units.length}</span>
          </button>
          <button className={panel === "chat" ? "active" : ""} onClick={() => setPanel(panel === "chat" ? null : "chat")}>
            Chat {unread > 0 && <span className="badge">{unread}</span>}
          </button>
        </div>
      </header>

      {mode !== "none" && (
        <div className="hint">
          {mode === "place-me" ? "Click the map to set your position" : "Click the map to drop a marker"}
          <button onClick={() => setMode("none")}>Cancel</button>
        </div>
      )}

      {panel === "contacts" && <ContactsPanel units={units} now={now} myUid={identity?.uid} onPick={focus} />}
      {panel === "chat" && (
        <ChatPanel
          chats={state.chats}
          myUid={identity?.uid}
          room={room}
          rooms={rooms}
          disabled={status !== "open" || !identity}
          onRoom={setRoom}
          onSend={sendChat}
        />
      )}

      {selectedUnit && (
        <DetailCard
          unit={selectedUnit}
          now={now}
          onClose={() => setSelected(null)}
          onDelete={() => {
            send({ op: "delete", uid: selectedUnit.uid });
            setSelected(null);
          }}
          onMessage={
            selectedUnit.callsign && !isMarker(selectedUnit) && selectedUnit.uid !== identity?.uid
              ? () => {
                  setRoom({ key: selectedUnit.uid, name: selectedUnit.callsign! });
                  setPanel("chat");
                  setSelected(null);
                }
              : undefined
          }
        />
      )}

      {draft && identity && (
        <MarkerDialog
          draft={draft}
          onCancel={() => setDraft(null)}
          onSubmit={(m) => {
            send({ op: "marker", uid: `${identity.uid}.${randomId()}`, ...m });
            setDraft(null);
          }}
        />
      )}

      {editingIdentity && (
        <SettingsDialog
          initial={identity}
          onSave={(id) => {
            saveIdentity(id);
            setIdentity(id);
            setEditingIdentity(false);
          }}
          onCancel={identity ? () => setEditingIdentity(false) : undefined}
        />
      )}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
