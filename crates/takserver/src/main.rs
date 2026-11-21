use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use takserver::{Config, mesh::MESH_GROUP};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// A small TAK server: relays CoT between ATAK clients, the mesh, and the web UI.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Plain TAK streaming port. Unauthenticated, like TAK Server's 8087: keep it on trusted networks.
    #[arg(long, default_value = "0.0.0.0:8087")]
    tcp: SocketAddr,
    /// Web UI and WebSocket.
    #[arg(long, default_value = "127.0.0.1:8080")]
    http: SocketAddr,
    /// Built web UI to serve.
    #[arg(long, default_value = "web/dist")]
    web_root: PathBuf,
    /// Bridge the UDP mesh multicast group.
    #[arg(long)]
    mesh: bool,
    #[arg(long, default_value_t = MESH_GROUP)]
    mesh_group: SocketAddrV4,
    /// IPv4 address of the interface to use for multicast.
    #[arg(long)]
    iface: Option<Ipv4Addr>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("takserver=info")),
        )
        .init();
    let cli = Cli::parse();

    let cancel = CancellationToken::new();
    let config = Config {
        tcp: cli.tcp,
        http: cli.http,
        web_root: Some(cli.web_root),
        mesh: cli.mesh.then_some((cli.mesh_group, cli.iface)),
    };
    let server = takserver::start(config, cancel.clone()).await?;
    info!(tak = %server.tcp_addr, http = %server.http_addr, mesh = cli.mesh, "listening");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
        _ = cancel.cancelled() => {}
    }
    cancel.cancel();
    server.wait().await;
    Ok(())
}
