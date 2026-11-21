//! Bridges the UDP mesh: phones on the local network that aren't connected to any server see
//! server clients, and vice versa.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use cot::framing;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::hub::{Hub, PING_TYPE, Source};

pub const MESH_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 2, 3, 1), 6969);

pub fn bind(group: SocketAddrV4, iface: Option<Ipv4Addr>) -> io::Result<(UdpSocket, UdpSocket)> {
    let iface = iface.unwrap_or(Ipv4Addr::UNSPECIFIED);

    let recv = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    recv.set_reuse_address(true)?;
    #[cfg(unix)]
    recv.set_reuse_port(true)?;
    recv.set_nonblocking(true)?;
    recv.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, group.port())).into())?;
    recv.join_multicast_v4(group.ip(), &iface)?;

    let send = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    send.set_multicast_ttl_v4(1)?;
    // Our own sends must not come back in through `recv` and be relayed again.
    send.set_multicast_loop_v4(false)?;
    if !iface.is_unspecified() {
        send.set_multicast_if_v4(&iface)?;
    }
    send.set_nonblocking(true)?;
    send.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;

    Ok((UdpSocket::from_std(recv.into())?, UdpSocket::from_std(send.into())?))
}

pub async fn bridge(
    recv: UdpSocket,
    send: UdpSocket,
    group: SocketAddrV4,
    hub: Arc<Hub>,
    cancel: CancellationToken,
) {
    let mut rx = hub.subscribe();
    let outbound = async {
        loop {
            let env = match rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("mesh sender skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            // Multicast reaches everyone, so addressed events (direct chat) would leak to the room.
            if env.source == Source::Mesh || env.event.event_type == PING_TYPE || !env.dest.is_empty() {
                continue;
            }
            if let Err(e) = send.send_to(env.xml.as_bytes(), group).await {
                warn!("mesh send failed: {e}");
            }
        }
    };
    let inbound = async {
        let mut buf = vec![0u8; 65_536];
        loop {
            let (n, from) = match recv.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("mesh receive failed: {e}");
                    continue;
                }
            };
            let msg = match framing::decode_datagram(&buf[..n]).and_then(|f| f.decode()) {
                Ok(m) => m,
                Err(e) => {
                    debug!(%from, "dropping mesh datagram: {e}");
                    continue;
                }
            };
            if let Some(ev) = msg.event
                && ev.event_type != PING_TYPE
            {
                hub.publish(Source::Mesh, ev);
            }
        }
    };
    tokio::select! {
        _ = outbound => {}
        _ = inbound => {}
        _ = cancel.cancelled() => {}
    }
}
