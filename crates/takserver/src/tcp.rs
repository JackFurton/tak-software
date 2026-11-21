//! Streaming clients (8087 plain, 8089 TLS): each sends CoT and receives everyone else's. A
//! connection starts as XML and moves to protobuf if the client accepts the server's offer.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use cot::framing::{self, StreamDecoder};
use cot::{Event, Point, takp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::hub::{Envelope, Hub, PING_TYPE, Peer, Source};

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

/// Sent from the reader to the writer, ahead of the broadcast queue.
enum Direct {
    Send(Event),
    /// Written as this connection's last XML frame; everything after it is protobuf.
    SwitchToProtobuf(Event),
}

async fn client(stream: TcpStream, peer: SocketAddr, id: u64, hub: Arc<Hub>, cancel: CancellationToken) {
    let _ = stream.set_nodelay(true);
    info!(%peer, id, "client connected");
    let (reader, writer) = stream.into_split();
    let (direct_tx, direct_rx) = mpsc::channel(8);
    let (who_tx, who_rx) = watch::channel(None);
    // Subscribe before taking the snapshot so nothing published in between is missed.
    let rx = hub.subscribe();
    let writer = tokio::spawn(write_loop(writer, rx, direct_rx, who_rx, hub.clone(), id, cancel.clone()));

    read_loop(reader, &hub, id, direct_tx, &who_tx, &cancel).await;
    cancel.cancel();
    let _ = writer.await;
    let callsign = who_tx.borrow().as_ref().map(|p: &Peer| p.callsign.clone());
    info!(%peer, id, callsign = callsign.as_deref().unwrap_or("-"), "client disconnected");
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    hub: &Hub,
    id: u64,
    direct: mpsc::Sender<Direct>,
    who: &watch::Sender<Option<Peer>>,
    cancel: &CancellationToken,
) {
    let mut decoder = StreamDecoder::default();
    let mut buf = vec![0u8; 16 * 1024];
    let mut protobuf = false;
    loop {
        let n = tokio::select! {
            r = tokio::time::timeout(IDLE_TIMEOUT, reader.read(&mut buf)) => match r {
                Ok(Ok(0)) => return,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    debug!(id, "read failed: {e}");
                    return;
                }
                Err(_) => {
                    warn!(id, "idle for {IDLE_TIMEOUT:?}, dropping");
                    return;
                }
            },
            _ = cancel.cancelled() => return,
        };
        decoder.extend(&buf[..n]);
        loop {
            let frame = match decoder.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    warn!(id, "unreadable stream, dropping: {e}");
                    return;
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
                let _ = direct.try_send(Direct::Send(pong()));
                continue;
            }
            if takp::is_negotiation(&ev) {
                if let Some(version) = takp::requested_version(&ev) {
                    let accept = version == takp::VERSION && !protobuf;
                    let reply = takp::response(&ev.uid, accept, Utc::now());
                    info!(id, version, accept, "protocol negotiation");
                    protobuf |= accept;
                    let _ = direct
                        .send(if accept { Direct::SwitchToProtobuf(reply) } else { Direct::Send(reply) })
                        .await;
                }
                continue;
            }
            identify(&ev, who, id);
            hub.publish(Source::Tcp(id), ev);
        }
    }
}

/// The first atom carrying a callsign is the client's own SA. Later ones from the same uid keep
/// the callsign current, which `<marti>` routing matches on.
fn identify(ev: &Event, who: &watch::Sender<Option<Peer>>, id: u64) {
    let Some(contact) = &ev.detail.contact else { return };
    if !ev.cot_type().is_atom() {
        return;
    }
    who.send_if_modified(|current| match current {
        None => {
            info!(id, uid = %ev.uid, callsign = %contact.callsign, "client identified");
            *current = Some(Peer { uid: ev.uid.clone(), callsign: contact.callsign.clone() });
            true
        }
        Some(p) if p.uid == ev.uid && p.callsign != contact.callsign => {
            p.callsign = contact.callsign.clone();
            true
        }
        _ => false,
    });
}

async fn write_loop(
    mut writer: OwnedWriteHalf,
    mut rx: broadcast::Receiver<Arc<Envelope>>,
    mut direct: mpsc::Receiver<Direct>,
    who: watch::Receiver<Option<Peer>>,
    hub: Arc<Hub>,
    id: u64,
    cancel: CancellationToken,
) {
    let me = Source::Tcp(id);
    let now = Utc::now();
    let offer = takp::support(&format!("takserver-{id}"), &[takp::VERSION], now).to_xml();
    if !write(&mut writer, offer.as_bytes(), id).await {
        return;
    }
    // New clients get the current picture: live units and markers, not old chat or deletes.
    for env in hub.snapshot() {
        let wanted = env.source != me
            && env.event.cot_type().is_atom()
            && !env.event.is_stale(now)
            && env.visible_to(None);
        if wanted && !write(&mut writer, env.stream_bytes(false), id).await {
            return;
        }
    }
    let mut protobuf = false;
    loop {
        let bytes = tokio::select! {
            r = rx.recv() => match r {
                Ok(env) if env.source == me || !env.visible_to(who.borrow().as_ref()) => continue,
                Ok(env) => env.stream_bytes(protobuf).to_vec(),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(id, "client too slow, skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            Some(d) = direct.recv() => match d {
                Direct::Send(ev) => encode(&ev, protobuf),
                Direct::SwitchToProtobuf(ev) => {
                    let bytes = encode(&ev, false);
                    protobuf = true;
                    bytes
                }
            },
            _ = cancel.cancelled() => break,
        };
        if !write(&mut writer, &bytes, id).await {
            break;
        }
    }
    cancel.cancel();
    let _ = writer.shutdown().await;
}

fn encode(ev: &Event, protobuf: bool) -> Vec<u8> {
    if protobuf { framing::encode_stream(&ev.to_tak_message()) } else { ev.to_xml().into_bytes() }
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
