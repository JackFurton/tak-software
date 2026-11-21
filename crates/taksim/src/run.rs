use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::sink::{Mesh, StreamClient};
use crate::unit::Unit;

pub struct Outputs {
    pub mesh: Option<Arc<Mesh>>,
    /// Each unit opens its own connection, like a fleet of separate phones.
    pub tcp: Option<String>,
    pub stdout: bool,
}

/// Runs every unit until `cancel` fires, then sends each a final report that is already stale so
/// clients drop the icons instead of waiting out the stale window.
pub async fn run(units: Vec<Unit>, interval: Duration, outputs: Outputs, cancel: CancellationToken) {
    let started = Instant::now();
    let stale_after = (interval * 4).max(Duration::from_secs(30));
    let outputs = Arc::new(outputs);
    let count = units.len() as u32;

    let mut tasks = JoinSet::new();
    for (i, unit) in units.into_iter().enumerate() {
        let outputs = outputs.clone();
        let cancel = cancel.clone();
        // Spread units across the interval so a big swarm doesn't send in one burst.
        let offset = interval * i as u32 / count.max(1);
        tasks.spawn(async move {
            let tcp =
                outputs.tcp.clone().map(|addr| StreamClient::spawn(addr, unit.uid.clone(), cancel.clone()));
            let mut ticker = tokio::time::interval_at(started + offset, interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                let last = tokio::select! {
                    _ = ticker.tick() => false,
                    _ = cancel.cancelled() => true,
                };
                let now = Utc::now();
                let mut ev = unit.report(started.elapsed(), now, stale_after);
                if last {
                    ev.stale = now;
                }
                emit(&outputs, tcp.as_ref(), &ev).await;
                if last {
                    break;
                }
            }
            if let Some(tcp) = tcp {
                tcp.close().await;
            }
        });
    }
    while tasks.join_next().await.is_some() {}
}

async fn emit(outputs: &Outputs, tcp: Option<&StreamClient>, ev: &cot::Event) {
    if let Some(mesh) = &outputs.mesh
        && let Err(e) = mesh.send(ev).await
    {
        warn!(uid = %ev.uid, "mesh send failed: {e}");
    }
    if let Some(tcp) = tcp {
        tcp.send(ev);
    }
    if outputs.stdout {
        println!("{}", ev.to_xml());
    }
}
