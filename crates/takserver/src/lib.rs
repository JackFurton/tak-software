pub mod hub;
pub mod mesh;
pub mod tcp;
pub mod web;
pub mod wire;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub struct Config {
    pub tcp: SocketAddr,
    pub http: SocketAddr,
    pub web_root: Option<PathBuf>,
    pub mesh: Option<(SocketAddrV4, Option<Ipv4Addr>)>,
}

pub struct Server {
    pub tcp_addr: SocketAddr,
    pub http_addr: SocketAddr,
    pub hub: Arc<hub::Hub>,
    tasks: JoinSet<()>,
}

impl Server {
    pub async fn wait(mut self) {
        while self.tasks.join_next().await.is_some() {}
    }
}

/// Binds every listener up front, so a port conflict fails startup instead of a background task.
pub async fn start(config: Config, cancel: CancellationToken) -> anyhow::Result<Server> {
    let hub = Arc::new(hub::Hub::default());
    let tcp =
        TcpListener::bind(config.tcp).await.with_context(|| format!("binding TAK port {}", config.tcp))?;
    let http =
        TcpListener::bind(config.http).await.with_context(|| format!("binding HTTP {}", config.http))?;
    let mut tasks = JoinSet::new();

    if let Some((group, iface)) = config.mesh {
        let (recv, send) = mesh::bind(group, iface).context("joining mesh multicast group")?;
        tasks.spawn(mesh::bridge(recv, send, group, hub.clone(), cancel.clone()));
    }

    let (tcp_addr, http_addr) = (tcp.local_addr()?, http.local_addr()?);

    tasks.spawn(tcp::serve(tcp, hub.clone(), cancel.clone()));
    let app = web::router(hub.clone(), config.web_root, cancel.clone());
    let shutdown = cancel.clone();
    tasks.spawn(async move {
        if let Err(e) = axum::serve(http, app).with_graceful_shutdown(shutdown.cancelled_owned()).await {
            tracing::error!("HTTP server failed: {e}");
        }
    });
    tasks.spawn({
        let hub = hub.clone();
        async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = tick.tick() => hub.purge(chrono::Utc::now()),
                    _ = cancel.cancelled() => return,
                }
            }
        }
    });

    Ok(Server { tcp_addr, http_addr, hub, tasks })
}
