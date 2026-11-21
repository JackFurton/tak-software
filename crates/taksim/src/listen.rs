use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use cot::framing::{self, Frame};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Joins a mesh group and prints one line per event, or the raw XML with `raw`.
pub async fn listen(
    group: SocketAddrV4,
    iface: Ipv4Addr,
    raw: bool,
    cancel: CancellationToken,
) -> io::Result<()> {
    let socket = join(group, iface)?;
    let mut buf = vec![0u8; 65_536];
    loop {
        let (n, from) = tokio::select! {
            r = socket.recv_from(&mut buf) => r?,
            _ = cancel.cancelled() => return Ok(()),
        };
        let frame = match framing::decode_datagram(&buf[..n]) {
            Ok(f) => f,
            Err(e) => {
                warn!(%from, "undecodable datagram: {e}");
                continue;
            }
        };
        let format = if matches!(frame, Frame::Xml(_)) { "xml" } else { "pb " };
        match frame.decode() {
            Ok(msg) => {
                let Some(ev) = msg.event else { continue };
                if raw {
                    println!("{}", ev.to_xml());
                } else {
                    println!("{format} {from:<21} {}", summarize(&ev));
                }
            }
            Err(e) => warn!(%from, "bad event: {e}"),
        }
    }
}

fn join(group: SocketAddrV4, iface: Ipv4Addr) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // ATAK, WinTAK and other listeners on the same host bind this port too.
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, group.port())).into())?;
    socket.join_multicast_v4(group.ip(), &iface)?;
    UdpSocket::from_std(socket.into())
}

fn summarize(ev: &cot::Event) -> String {
    let callsign = ev.detail.contact.as_ref().map_or("-", |c| c.callsign.as_str());
    let label = ev.cot_type().label().unwrap_or_default();
    let motion = ev
        .detail
        .track
        .as_ref()
        .map(|t| format!(" {:.1} m/s @ {:03.0}°", t.speed, t.course))
        .unwrap_or_default();
    format!(
        "{callsign:<12} {:<14} {label:<16} {:>10.5} {:>11.5}{motion}",
        ev.event_type, ev.point.lat, ev.point.lon
    )
}
