use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use chrono::Utc;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

use crate::hub::{Hub, Peer, Source};
use crate::wire::{ClientOp, WebSession};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const MAX_MESSAGE: usize = 16 * 1024;
// A UI sends an SA every few seconds plus the odd chat or marker; anything near this is a bug
// or abuse.
const MAX_OPS_PER_SEC: u32 = 20;

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    cancel: CancellationToken,
}

pub fn router(hub: Arc<Hub>, web_root: Option<PathBuf>, cancel: CancellationToken) -> Router {
    let api = Router::new().route("/ws", get(ws)).route("/api/health", get(health));
    let app = match web_root.filter(|root| root.join("index.html").is_file()) {
        Some(root) => {
            let index = root.join("index.html");
            api.fallback_service(ServeDir::new(root).fallback(ServeFile::new(index)))
        }
        None => api.fallback(|| async {
            "takserver is running, but the web UI isn't built. Run `npm install && npm run build` in web/."
        }),
    };
    app.with_state(AppState { hub, cancel })
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(
        serde_json::json!({ "ok": true, "tracked": state.hub.tracked(), "version": env!("CARGO_PKG_VERSION") }),
    )
}

async fn ws(upgrade: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    upgrade.max_message_size(MAX_MESSAGE).on_upgrade(move |socket| session(socket, state))
}

async fn session(socket: WebSocket, state: AppState) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.hub.subscribe();
    let mut web = WebSession::default();
    info!(id, "web client connected");

    // Addressed events wait for hello, when we know who this is.
    for env in state.hub.snapshot().iter().filter(|e| e.visible_to(None)) {
        if sink.send(Message::Text(env.json.clone().into())).await.is_err() {
            return;
        }
    }

    let mut peer = None;
    let mut window = (Instant::now(), 0u32);
    loop {
        let out = tokio::select! {
            msg = stream.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                if window.0.elapsed() >= Duration::from_secs(1) {
                    window = (Instant::now(), 0);
                }
                window.1 += 1;
                if window.1 > MAX_OPS_PER_SEC {
                    Some(error_json("rate limited"))
                } else {
                    match serde_json::from_str::<ClientOp>(&text)
                        .map_err(|e| e.to_string())
                        .and_then(|op| web.handle(op, Utc::now()))
                    {
                        Ok(Some(ev)) => {
                            state.hub.publish(Source::Web(id), ev);
                            None
                        }
                        Ok(None) => {
                            let now = web.peer();
                            if now != peer {
                                peer = now;
                                if !replay_addressed(&mut sink, &state.hub, peer.as_ref()).await {
                                    break;
                                }
                            }
                            None
                        }
                        Err(e) => Some(error_json(&e)),
                    }
                }
            }
            env = rx.recv() => match env {
                Ok(env) if env.source == Source::Web(id) || env.visible_to(peer.as_ref()) => {
                    Some(env.json.clone())
                }
                Ok(_) => None,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(id, "web client too slow, skipped {n} events");
                    None
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = state.cancel.cancelled() => break,
        };
        if let Some(json) = out
            && sink.send(Message::Text(json.into())).await.is_err()
        {
            break;
        }
    }

    if let Some(ev) = web.farewell(Utc::now()) {
        state.hub.publish(Source::Web(id), ev);
    }
    info!(id, uid = web.uid().unwrap_or("-"), "web client disconnected");
}

/// Sends addressed events `peer` may see, such as direct chats from before it connected.
async fn replay_addressed(sink: &mut SplitSink<WebSocket, Message>, hub: &Hub, peer: Option<&Peer>) -> bool {
    for env in hub.snapshot().iter().filter(|e| !e.dest.is_empty() && e.visible_to(peer)) {
        if sink.send(Message::Text(env.json.clone().into())).await.is_err() {
            return false;
        }
    }
    true
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "kind": "error", "message": message }).to_string()
}
