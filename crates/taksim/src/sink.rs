use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use cot::Event;
use cot::framing::{self, StreamDecoder};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub const MESH_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 2, 3, 1), 6969);

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MeshFormat {
    Xml,
    Protobuf,
}

/// Sends every report to a multicast group, as ATAK does with mesh SA.
pub struct Mesh {
    socket: UdpSocket,
    dest: SocketAddr,
    format: MeshFormat,
}

impl Mesh {
    /// `iface` picks the outgoing interface by its IPv4 address; hosts with several (WSL, VPNs,
    /// Docker bridges) otherwise send wherever the default route points.
    pub fn new(
        dest: SocketAddrV4,
        iface: Option<Ipv4Addr>,
        ttl: u32,
        format: MeshFormat,
    ) -> io::Result<Self> {
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
            MeshFormat::Xml => ev.to_xml().into_bytes(),
            MeshFormat::Protobuf => framing::encode_mesh(&ev.to_tak_message()),
        };
        self.socket.send_to(&bytes, self.dest).await.map(drop)
    }
}

/// One simulated device's connection to a TAK server, run on its own task so connecting and
/// backoff never delay the unit's reports or shutdown. Reports that pile up while disconnected
/// are dropped except the newest, since each one supersedes the last.
///
/// Always speaks XML: a server only switches a connection to protobuf after negotiating it, and
/// every server accepts XML.
pub struct StreamClient {
    tx: mpsc::Sender<Vec<u8>>,
    task: JoinHandle<()>,
}

const QUEUE: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// A server that stops reading would otherwise block shutdown forever.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

impl StreamClient {
    pub fn spawn(addr: String, uid: String, cancel: CancellationToken) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let task = tokio::spawn(connection(addr, uid, rx, cancel));
        Self { tx, task }
    }

    pub fn send(&self, ev: &Event) {
        if self.tx.try_send(ev.to_xml().into_bytes()).is_err() {
            debug!(uid = %ev.uid, "server connection backed up, dropping report");
        }
    }

    /// Flushes what's queued if connected, then closes. Gives up at once if not connected.
    pub async fn close(self) {
        drop(self.tx);
        let _ = self.task.await;
    }
}

async fn connection(addr: String, uid: String, mut rx: mpsc::Receiver<Vec<u8>>, cancel: CancellationToken) {
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
        tokio::spawn(drain(reader, uid.clone()));

        let mut pending = None;
        while let Ok(bytes) = rx.try_recv() {
            pending = Some(bytes);
        }
        loop {
            let bytes = match pending.take() {
                Some(b) => b,
                None => match rx.recv().await {
                    Some(b) => b,
                    None => {
                        let _ = writer.shutdown().await;
                        return;
                    }
                },
            };
            match tokio::time::timeout(WRITE_TIMEOUT, writer.write_all(&bytes)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    warn!(%uid, %addr, "write failed, reconnecting: {e}");
                    break;
                }
                Err(_) => {
                    warn!(%uid, %addr, "server stopped reading, reconnecting");
                    break;
                }
            }
        }
    }
}

// Servers relay everyone's traffic to every client. It has to be read or the socket buffer fills
// and the server stalls or drops us.
async fn drain(mut reader: tokio::net::tcp::OwnedReadHalf, uid: String) {
    let mut decoder = StreamDecoder::default();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => decoder.extend(&buf[..n]),
        }
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) => {
                    if let Ok(msg) = frame.decode()
                        && let Some(ev) = msg.event
                    {
                        debug!(%uid, from = %ev.uid, kind = %ev.event_type, "received");
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(%uid, "unreadable stream from server: {e}");
                    return;
                }
            }
        }
    }
    debug!(%uid, "server closed connection");
}
