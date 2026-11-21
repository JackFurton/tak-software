import { useCallback, useEffect, useRef, useState } from "react";
import type { ClientOp, Identity, ServerMessage } from "./types";

export type Status = "connecting" | "open" | "closed";

/** Keeps a WebSocket to the server open, re-introducing ourselves after every reconnect. */
export function useTakSocket(identity: Identity | null, onMessage: (msg: ServerMessage) => void) {
  const [status, setStatus] = useState<Status>("connecting");
  const ws = useRef<WebSocket | null>(null);
  const handler = useRef(onMessage);
  handler.current = onMessage;
  const me = useRef(identity);
  me.current = identity;

  const hello = useCallback((socket: WebSocket) => {
    const id = me.current;
    if (id && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ op: "hello", ...id }));
  }, []);

  useEffect(() => {
    let closed = false;
    let backoff = 1000;
    let timer: number | undefined;

    const connect = () => {
      setStatus("connecting");
      const scheme = location.protocol === "https:" ? "wss" : "ws";
      const socket = new WebSocket(`${scheme}://${location.host}/ws`);
      ws.current = socket;
      socket.onopen = () => {
        backoff = 1000;
        setStatus("open");
        hello(socket);
      };
      socket.onmessage = (e) => {
        try {
          handler.current(JSON.parse(e.data) as ServerMessage);
        } catch (err) {
          console.warn("bad message from server", err);
        }
      };
      socket.onclose = () => {
        if (closed) return;
        setStatus("closed");
        timer = window.setTimeout(connect, backoff);
        backoff = Math.min(backoff * 2, 10_000);
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(timer);
      ws.current?.close();
    };
  }, [hello]);

  useEffect(() => {
    if (ws.current) hello(ws.current);
  }, [identity, hello]);

  const send = useCallback((op: ClientOp) => {
    const socket = ws.current;
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(op));
  }, []);

  return { status, send };
}
