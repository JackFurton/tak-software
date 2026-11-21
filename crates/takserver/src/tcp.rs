//! Plain streaming port (8087): TAK clients send CoT XML and receive everyone else's.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use cot::framing::StreamDecoder;
use cot::{Event, Point};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::hub::{Hub, PING_TYPE, Source};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// ATAK reports at least every few minutes even when stationary; silence this long means a dead
// link that never sent FIN (phone left Wi-Fi).
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn serve(listener: TcpListener, hub: Arc<Hub>, cancel: CancellationToken) {
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(e) => {
                    warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            _ = cancel.cancelled() => return,
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(client(stream, peer, id, hub.clone(), cancel.child_token()));
    }
}

async fn client(stream: TcpStream, peer: SocketAddr, id: u64, hub: Arc<Hub>, cancel: CancellationToken) {
    let _ = stream.set_nodelay(true);
    info!(%peer, id, "client connected");
    let (reader, writer) = stream.into_split();
    let (direct_tx, direct_rx) = mpsc::channel(8);
    // Subscribe before taking the snapshot so nothing published in between is missed.
    let rx = hub.subscribe();
    let writer = tokio::spawn(write_loop(writer, rx, direct_rx, hub.clone(), id, cancel.clone()));

    let callsign = read_loop(reader, &hub, id, direct_tx, &cancel).await;
    cancel.cancel();
    let _ = writer.await;
    info!(%peer, id, callsign = callsign.as_deref().unwrap_or("-"), "client disconnected");
}

/// Returns the client's callsign, if it ever sent one.
async fn read_loop(
    mut reader: OwnedReadHalf,
    hub: &Hub,
    id: u64,
    direct: mpsc::Sender<String>,
    cancel: &CancellationToken,
) -> Option<String> {
    let mut decoder = StreamDecoder::default();
    let mut buf = vec![0u8; 16 * 1024];
    let mut callsign = None;
    loop {
        let n = tokio::select! {
            r = tokio::time::timeout(IDLE_TIMEOUT, reader.read(&mut buf)) => match r {
                Ok(Ok(0)) => return callsign,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    debug!(id, "read failed: {e}");
                    return callsign;
                }
                Err(_) => {
                    warn!(id, "idle for {IDLE_TIMEOUT:?}, dropping");
                    return callsign;
                }
            },
            _ = cancel.cancelled() => return callsign,
        };
        decoder.extend(&buf[..n]);
        loop {
            let frame = match decoder.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    warn!(id, "unreadable stream, dropping: {e}");
                    return callsign;
                }
            };
            let ev = match frame.decode() {
                Ok(msg) => match msg.event {
                    Some(ev) => ev,
                    None => continue,
                },
                Err(e) => {
                    warn!(id, "bad event: {e}");
                    continue;
                }
            };
            if ev.event_type == PING_TYPE {
                let _ = direct.try_send(pong().to_xml());
                continue;
            }
            // Protocol negotiation (t-x-takp-*): staying silent keeps the client on XML.
            if ev.event_type.starts_with("t-x-takp") {
                continue;
            }
            if callsign.is_none()
                && let Some(c) = &ev.detail.contact
                && ev.cot_type().is_atom()
            {
                info!(id, uid = %ev.uid, callsign = %c.callsign, "client identified");
                callsign = Some(c.callsign.clone());
            }
            hub.publish(Source::Tcp(id), ev);
        }
    }
}

async fn write_loop(
    mut writer: OwnedWriteHalf,
    mut rx: broadcast::Receiver<Arc<crate::hub::Envelope>>,
    mut direct: mpsc::Receiver<String>,
    hub: Arc<Hub>,
    id: u64,
    cancel: CancellationToken,
) {
    let me = Source::Tcp(id);
    let now = Utc::now();
    // New clients get the current picture: live units and markers, not old chat or deletes.
    for env in hub.snapshot() {
        let wanted = env.source != me && env.event.cot_type().is_atom() && !env.event.is_stale(now);
        if wanted && !write(&mut writer, env.xml.as_bytes(), id).await {
            return;
        }
    }
    loop {
        let bytes = tokio::select! {
            r = rx.recv() => match r {
                Ok(env) if env.source == me => continue,
                Ok(env) => env.xml.clone(),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(id, "client too slow, skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            Some(xml) = direct.recv() => xml,
            _ = cancel.cancelled() => break,
        };
        if !write(&mut writer, bytes.as_bytes(), id).await {
            break;
        }
    }
    cancel.cancel();
    let _ = writer.shutdown().await;
}

async fn write(writer: &mut OwnedWriteHalf, bytes: &[u8], id: u64) -> bool {
    match tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(bytes)).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            debug!(id, "write failed: {e}");
            false
        }
        Err(_) => {
            warn!(id, "client stopped reading, dropping");
            false
        }
    }
}

/// Reply to a client's `t-x-c-t` keepalive, in the form TAK Server sends.
fn pong() -> Event {
    let mut ev = Event::new("takPong", "t-x-c-t-r", Point { hae: 0.0, ..Point::new(0.0, 0.0) });
    ev.how = "h-g-i-g-o".into();
    ev.stale = ev.time + chrono::Duration::seconds(20);
    ev
}
