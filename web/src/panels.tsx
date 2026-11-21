import { useEffect, useRef, useState } from "react";
import { randomId } from "./ids";
import { ALL_ROOM, ROLES, TEAMS, TEAM_COLORS, roomOf, type Chat, type Identity, type Room, type TakEvent } from "./types";

function age(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return `${Math.floor(s / 3600)}h`;
}

export const isMarker = (u: TakEvent) => u.how.startsWith("h-g-i-g-o");

export function ContactsPanel(props: { units: TakEvent[]; now: number; myUid?: string; onPick: (u: TakEvent) => void }) {
  const devices = props.units.filter((u) => !isMarker(u)).sort((a, b) => (a.callsign ?? "").localeCompare(b.callsign ?? ""));
  const markers = props.units.filter(isMarker).sort((a, b) => b.time - a.time);
  const row = (u: TakEvent) => (
    <li key={u.uid} className={props.now >= u.stale ? "stale" : ""} onClick={() => props.onPick(u)}>
      <span className="team-dot" style={{ background: u.team ? TEAM_COLORS[u.team] : "transparent" }} />
      <span className="name">
        {u.callsign ?? u.uid}
        {u.uid === props.myUid && <em> (you)</em>}
      </span>
      <span className="meta">
        {u.battery !== undefined && `${u.battery}% · `}
        {age(props.now - u.time)}
      </span>
    </li>
  );
  return (
    <aside className="panel">
      <h2>Devices <span className="count">{devices.length}</span></h2>
      <ul className="list">{devices.map(row)}</ul>
      <h2>Markers <span className="count">{markers.length}</span></h2>
      <ul className="list">{markers.map(row)}</ul>
      {props.units.length === 0 && <p className="empty">Nothing on the map yet. Start taksim or connect ATAK.</p>}
    </aside>
  );
}

export function ChatPanel(props: {
  chats: Chat[];
  myUid?: string;
  room: Room;
  rooms: Room[];
  disabled: boolean;
  onRoom: (room: Room) => void;
  onSend: (text: string) => void;
}) {
  const [text, setText] = useState("");
  const end = useRef<HTMLDivElement>(null);
  const shown = props.chats.filter((c) => roomOf(c, props.myUid).key === props.room.key);
  // Braces matter: newer browsers return a Promise from scrollIntoView, which React would take
  // for a cleanup function and crash calling.
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [shown.length, props.room.key]);
  return (
    <aside className="panel chat">
      <nav className="rooms">
        {props.rooms.map((r) => (
          <button key={r.key} className={r.key === props.room.key ? "active" : ""} onClick={() => props.onRoom(r)}>
            {r.key === ALL_ROOM.key ? "All" : r.name}
          </button>
        ))}
      </nav>
      <h2>{props.room.key === ALL_ROOM.key ? ALL_ROOM.name : `Direct · ${props.room.name}`}</h2>
      <div className="messages">
        {shown.length === 0 && <p className="empty">No messages yet.</p>}
        {shown.map((c) => (
          <div key={c.id} className={`message ${c.sender_uid === props.myUid ? "mine" : ""}`}>
            <div className="from">
              {c.sender_callsign} · {new Date(c.time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
            </div>
            <div className="text">{c.text}</div>
          </div>
        ))}
        <div ref={end} />
      </div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!text.trim()) return;
          props.onSend(text.trim());
          setText("");
        }}
      >
        <input value={text} onChange={(e) => setText(e.target.value)} placeholder="Message" maxLength={2000} disabled={props.disabled} />
        <button disabled={props.disabled || !text.trim()}>Send</button>
      </form>
    </aside>
  );
}

export function DetailCard(props: {
  unit: TakEvent;
  now: number;
  onClose: () => void;
  onDelete: () => void;
  onMessage?: () => void;
}) {
  const u = props.unit;
  const rows: [string, string | undefined][] = [
    ["Type", u.label ? `${u.label} (${u.type})` : u.type],
    ["Team", u.team && `${u.team}${u.role ? ` · ${u.role}` : ""}`],
    ["Position", `${u.lat.toFixed(5)}, ${u.lon.toFixed(5)}`],
    ["Speed", u.speed !== undefined ? `${(u.speed * 3.6).toFixed(1)} km/h` : undefined],
    ["Course", u.course !== undefined ? `${Math.round(u.course)}°` : undefined],
    ["Battery", u.battery !== undefined ? `${u.battery}%` : undefined],
    ["Remarks", u.remarks],
    ["Updated", `${age(props.now - u.time)} ago via ${u.source}`],
    ["Stale", props.now >= u.stale ? "yes" : `in ${age(u.stale - props.now)}`],
    ["UID", u.uid],
  ];
  return (
    <section className="card">
      <header>
        <strong>{u.callsign ?? u.uid}</strong>
        <button className="close" onClick={props.onClose} aria-label="Close">×</button>
      </header>
      <dl>
        {rows.filter(([, v]) => v).map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {isMarker(u) && <button className="danger" onClick={props.onDelete}>Delete marker</button>}
      {props.onMessage && <button onClick={props.onMessage}>Message</button>}
    </section>
  );
}

export interface MarkerDraft {
  lat: number;
  lon: number;
}

const AFFILIATIONS = [
  ["h", "Hostile"],
  ["f", "Friend"],
  ["n", "Neutral"],
  ["u", "Unknown"],
] as const;
const DIMENSIONS = [
  ["G", "Ground"],
  ["A", "Air"],
  ["S", "Sea"],
] as const;

export function MarkerDialog(props: {
  draft: MarkerDraft;
  onCancel: () => void;
  onSubmit: (m: { type: string; name: string; lat: number; lon: number; remarks: string }) => void;
}) {
  const [aff, setAff] = useState<string>("h");
  const [dim, setDim] = useState<string>("G");
  const [name, setName] = useState("");
  const [remarks, setRemarks] = useState("");
  const label = AFFILIATIONS.find(([c]) => c === aff)![1];
  return (
    <div className="dialog-backdrop">
      <form
        className="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          props.onSubmit({
            type: `a-${aff}-${dim}`,
            name: name.trim() || `${label} ${randomId().slice(0, 4).toUpperCase()}`,
            lat: props.draft.lat,
            lon: props.draft.lon,
            remarks,
          });
        }}
      >
        <h2>New marker</h2>
        <div className="segmented">
          {AFFILIATIONS.map(([code, text]) => (
            <button type="button" key={code} className={`aff-${code} ${aff === code ? "active" : ""}`} onClick={() => setAff(code)}>
              {text}
            </button>
          ))}
        </div>
        <div className="segmented">
          {DIMENSIONS.map(([code, text]) => (
            <button type="button" key={code} className={dim === code ? "active" : ""} onClick={() => setDim(code)}>
              {text}
            </button>
          ))}
        </div>
        <label>
          Name
          <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={`${label} 1`} maxLength={64} />
        </label>
        <label>
          Remarks
          <textarea value={remarks} onChange={(e) => setRemarks(e.target.value)} rows={2} maxLength={2000} />
        </label>
        <p className="coords">
          {props.draft.lat.toFixed(5)}, {props.draft.lon.toFixed(5)}
        </p>
        <footer>
          <button type="button" onClick={props.onCancel}>Cancel</button>
          <button className="primary">Drop</button>
        </footer>
      </form>
    </div>
  );
}

export function SettingsDialog(props: { initial: Identity | null; onSave: (id: Identity) => void; onCancel?: () => void }) {
  const [callsign, setCallsign] = useState(props.initial?.callsign ?? "");
  const [team, setTeam] = useState(props.initial?.team ?? "Cyan");
  const [role, setRole] = useState(props.initial?.role ?? "Team Member");
  return (
    <div className="dialog-backdrop">
      <form
        className="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          if (!callsign.trim()) return;
          props.onSave({ uid: props.initial?.uid ?? `WEB-${randomId()}`, callsign: callsign.trim(), team, role });
        }}
      >
        <h2>{props.initial ? "Your device" : "Join the network"}</h2>
        <p className="muted">This browser acts as a TAK device. Others see you by callsign once you place yourself on the map.</p>
        <label>
          Callsign
          <input autoFocus value={callsign} onChange={(e) => setCallsign(e.target.value)} placeholder="KESTREL" maxLength={64} />
        </label>
        <label>
          Team
          <select value={team} onChange={(e) => setTeam(e.target.value)}>
            {TEAMS.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        </label>
        <label>
          Role
          <select value={role} onChange={(e) => setRole(e.target.value)}>
            {ROLES.map((r) => (
              <option key={r}>{r}</option>
            ))}
          </select>
        </label>
        <footer>
          {props.onCancel && <button type="button" onClick={props.onCancel}>Cancel</button>}
          <button className="primary" disabled={!callsign.trim()}>Save</button>
        </footer>
      </form>
    </div>
  );
}
