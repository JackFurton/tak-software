use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use chrono::Utc;
use cot::framing::{self, StreamDecoder};
use cot::{Event, takp};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub const MESH_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 2, 3, 1), 6969);

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Encoding {
    Xml,
    Protobuf,
}

/// Sends every report to a multicast group, as ATAK does with mesh SA.
pub struct Mesh {
    socket: UdpSocket,
    dest: SocketAddr,
    format: Encoding,
}

impl Mesh {
    /// `iface` picks the outgoing interface by its IPv4 address; hosts with several (WSL, VPNs,
    /// Docker bridges) otherwise send wherever the default route points.
    pub fn new(dest: SocketAddrV4, iface: Option<Ipv4Addr>, ttl: u32, format: Encoding) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_multicast_ttl_v4(ttl)?;
        socket.set_multicast_loop_v4(true)?;
        if let Some(addr) = iface {
            socket.set_multicast_if_v4(&addr)?;
        }
        socket.set_nonblocking(true)?;
        socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
        Ok(Self { socket: UdpSocket::from_std(socket.into())?, dest: dest.into(), format })
    }

    pub async fn send(&self, ev: &Event) -> io::Result<()> {
        let bytes = match self.format {
            Encoding::Xml => ev.to_xml().into_bytes(),
            Encoding::Protobuf => framing::encode_mesh(&ev.to_tak_message()),
        };
        self.socket.send_to(&bytes, self.dest).await.map(drop)
    }
}

/// One unit's connection to a TAK server, run on its own task so slow connects and reconnect
/// backoff never delay the unit's reports or shutdown. Reports that pile up while disconnected
/// are dropped except the newest, since each one supersedes the last.
///
/// Starts on XML, as every server expects. With [`Encoding::Protobuf`] it accepts a server's
/// protocol offer and switches, as ATAK does.
pub struct StreamClient {
    tx: mpsc::Sender<Event>,
    task: JoinHandle<()>,
}

const QUEUE: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// A server that stops reading would otherwise block shutdown forever.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
// protocol.txt: a client with no answer after a minute must drop the connection.
const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(60);

impl StreamClient {
    pub fn spawn(addr: String, uid: String, encoding: Encoding, cancel: CancellationToken) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let task = tokio::spawn(connection(addr, uid, rx, encoding, cancel));
        Self { tx, task }
    }

    pub fn send(&self, ev: &Event) {
        if self.tx.try_send(ev.clone()).is_err() {
            debug!(uid = %ev.uid, "server connection backed up, dropping report");
        }
    }

    /// Flushes what's queued if connected, then closes. Gives up at once if not connected.
    pub async fn close(self) {
        drop(self.tx);
        let _ = self.task.await;
    }
}

/// What the reader saw that the writer has to act on.
enum Negotiation {
    Offered { uid: String, version_ok: bool },
    Answered(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wire {
    Xml,
    /// Request sent; the client may not send anything until the server answers.
    Waiting,
    Protobuf,
}

async fn connection(
    addr: String,
    uid: String,
    mut rx: mpsc::Receiver<Event>,
    encoding: Encoding,
    cancel: CancellationToken,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let attempt = tokio::select! {
            r = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr)) => r,
            _ = cancel.cancelled() => return,
        };
        let stream = match attempt {
            Ok(Ok(s)) => s,
            failed => {
                let reason = failed.map_or("timed out".into(), |r| r.unwrap_err().to_string());
                warn!(%uid, %addr, "connect failed ({reason}), retrying in {backoff:?}");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = cancel.cancelled() => return,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        info!(%uid, %addr, "connected");
        backoff = Duration::from_secs(1);
        let (reader, mut writer) = stream.into_split();
        let (neg_tx, mut neg_rx) = mpsc::channel(4);
        tokio::spawn(drain(reader, uid.clone(), neg_tx));

        let mut pending = None;
        while let Ok(ev) = rx.try_recv() {
            pending = Some(ev);
        }
        let mut wire = Wire::Xml;
        let mut deadline = Instant::now();
        loop {
            if wire != Wire::Waiting
                && let Some(ev) = pending.take()
            {
                let bytes = match wire {
                    Wire::Protobuf => framing::encode_stream(&ev.to_tak_message()),
                    _ => ev.to_xml().into_bytes(),
                };
                if !write(&mut writer, &bytes, &uid, &addr).await {
                    break;
                }
                continue;
            }
            tokio::select! {
                msg = neg_rx.recv() => match msg {
                    // The reader only stops when the server hangs up.
                    None => break,
                    Some(Negotiation::Offered { uid: nid, version_ok }) => {
                        if encoding == Encoding::Protobuf && version_ok && wire == Wire::Xml {
                            let ask = takp::request(&nid, takp::VERSION, Utc::now()).to_xml();
                            if !write(&mut writer, ask.as_bytes(), &uid, &addr).await {
                                break;
                            }
                            wire = Wire::Waiting;
                            deadline = Instant::now() + NEGOTIATION_TIMEOUT;
                        }
                    }
                    Some(Negotiation::Answered(accepted)) if wire == Wire::Waiting => {
                        info!(%uid, accepted, "protocol negotiation answered");
                        wire = if accepted { Wire::Protobuf } else { Wire::Xml };
                    }
                    Some(Negotiation::Answered(_)) => {}
                },
                ev = rx.recv() => match ev {
                    Some(ev) => pending = Some(ev),
                    None => {
                        let _ = writer.shutdown().await;
                        return;
                    }
                },
                _ = tokio::time::sleep_until(deadline), if wire == Wire::Waiting => {
                    warn!(%uid, %addr, "server never answered the protocol request, reconnecting");
                    break;
                }
            }
        }
    }
}

async fn write(writer: &mut OwnedWriteHalf, bytes: &[u8], uid: &str, addr: &str) -> bool {
    match tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(bytes)).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            warn!(%uid, %addr, "write failed, reconnecting: {e}");
            false
        }
        Err(_) => {
            warn!(%uid, %addr, "server stopped reading, reconnecting");
            false
        }
    }
}

// Servers relay everyone's traffic to every client. It has to be read or the socket buffer fills
// and the server stalls or drops us.
async fn drain(mut reader: OwnedReadHalf, uid: String, negotiation: mpsc::Sender<Negotiation>) {
    let mut decoder = StreamDecoder::default();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => decoder.extend(&buf[..n]),
        }
        loop {
            let frame = match decoder.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(e) => {
                    warn!(%uid, "unreadable stream from server: {e}");
                    return;
                }
            };
            let Some(ev) = frame.decode().ok().and_then(|m| m.event) else { continue };
            let note = if let Some(accepted) = takp::response_status(&ev) {
                Some(Negotiation::Answered(accepted))
            } else if ev.event_type == takp::SUPPORT_TYPE {
                let version_ok = takp::supported_versions(&ev).contains(&takp::VERSION);
                Some(Negotiation::Offered { uid: ev.uid.clone(), version_ok })
            } else {
                debug!(%uid, from = %ev.uid, kind = %ev.event_type, "received");
                None
            };
            if let Some(note) = note
                && negotiation.send(note).await.is_err()
            {
                return;
            }
        }
    }
    debug!(%uid, "server closed connection");
}
