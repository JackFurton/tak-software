use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use takserver::{Config, mesh::MESH_GROUP};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// A small TAK server: relays CoT between ATAK clients, the mesh, and the web UI.
#[derive(Parser)]
#[command(version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    serve: Serve,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the certificate authority behind the TLS port.
    #[command(subcommand)]
    Certs(Certs),
}

#[derive(Subcommand)]
enum Certs {
    /// Create the CA and the server's certificate.
    Init {
        #[arg(long, default_value = "certs")]
        dir: PathBuf,
        /// Name or IP clients use to reach the server; repeat for several.
        #[arg(long = "host", required = true)]
        hosts: Vec<String>,
    },
    /// Issue a device certificate and write an ATAK data package that connects with it.
    Client {
        name: String,
        #[arg(long, default_value = "certs")]
        dir: PathBuf,
        /// Where ATAK should connect, as host:port.
        #[arg(long)]
        connect: String,
    },
}

#[derive(Args)]
struct Serve {
    /// Plain TAK streaming port. Unauthenticated, like TAK Server's 8087: keep it on trusted networks.
    #[arg(long, default_value = "0.0.0.0:8087")]
    tcp: SocketAddr,
    /// TLS streaming port; clients need a certificate from `takserver certs client`.
    #[arg(long)]
    tls: Option<SocketAddr>,
    /// Directory written by `takserver certs init`.
    #[arg(long, default_value = "certs")]
    certs: PathBuf,
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
    match cli.command {
        Some(Command::Certs(Certs::Init { dir, hosts })) => {
            takserver::certs::init(&dir, &hosts)?;
            println!("CA and server certificate written to {}", dir.display());
            Ok(())
        }
        Some(Command::Certs(Certs::Client { name, dir, connect })) => {
            let path = takserver::certs::client(&dir, &name, &connect)?;
            println!("{}: import into ATAK (password {})", path.display(), takserver::certs::P12_PASSWORD);
            Ok(())
        }
        None => serve(cli.serve).await,
    }
}

async fn serve(cli: Serve) -> Result<()> {
    let cancel = CancellationToken::new();
    let config = Config {
        tcp: cli.tcp,
        tls: cli.tls.map(|addr| (addr, cli.certs)),
        http: cli.http,
        web_root: Some(cli.web_root),
        mesh: cli.mesh.then_some((cli.mesh_group, cli.iface)),
    };
    let server = takserver::start(config, cancel.clone()).await?;
    info!(tak = %server.tcp_addr, tls = ?server.tls_addr, http = %server.http_addr, mesh = cli.mesh, "listening");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
        _ = cancel.cancelled() => {}
    }
    cancel.cancel();
    server.wait().await;
    Ok(())
}
