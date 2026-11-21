use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use taksim::geo::LatLon;
use taksim::run::{Outputs, run};
use taksim::sink::{Encoding, MESH_GROUP, Mesh};
use taksim::unit::{Scenario, swarm};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Simulated TAK devices that report position like ATAK phones.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run simulated units. Without --scenario or --swarm, runs the built-in demo.
    Run(RunArgs),
    /// Print events seen on the mesh multicast group.
    Listen(ListenArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Scenario TOML (see scenarios/demo.toml).
    #[arg(long, conflicts_with = "swarm")]
    scenario: Option<PathBuf>,
    /// Generate this many units circling --center instead of loading a scenario.
    #[arg(long)]
    swarm: Option<usize>,
    /// Swarm center as LAT,LON.
    #[arg(long, default_value = "38.8895,-77.0353", value_parser = parse_latlon)]
    center: LatLon,
    /// Largest swarm circle radius, meters.
    #[arg(long, default_value_t = 3_000.0)]
    radius: f64,
    /// Seconds between reports; overrides the scenario.
    #[arg(long)]
    interval: Option<f64>,
    /// Stop after this many seconds.
    #[arg(long)]
    duration: Option<f64>,

    /// Send to the mesh multicast group.
    #[arg(long)]
    mesh: bool,
    #[arg(long, value_enum, default_value_t = Encoding::Xml)]
    mesh_format: Encoding,
    #[command(flatten)]
    group: GroupArgs,
    /// Multicast TTL; 1 keeps traffic on the local network.
    #[arg(long, default_value_t = 1)]
    ttl: u32,
    /// Connect each unit to a TAK server's plain streaming port, e.g. 127.0.0.1:8087.
    #[arg(long)]
    tcp: Option<String>,
    /// `protobuf` accepts a server's offer to switch from XML, as ATAK does; `xml` declines it.
    #[arg(long, value_enum, default_value_t = Encoding::Protobuf)]
    tcp_format: Encoding,
    /// Print each event's XML.
    #[arg(long)]
    stdout: bool,
}

#[derive(Args)]
struct ListenArgs {
    #[command(flatten)]
    group: GroupArgs,
    /// Print full XML instead of one line per event.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct GroupArgs {
    #[arg(long, default_value_t = MESH_GROUP)]
    mesh_group: SocketAddrV4,
    /// IPv4 address of the interface to use for multicast.
    #[arg(long)]
    iface: Option<Ipv4Addr>,
}

fn parse_latlon(s: &str) -> Result<LatLon, String> {
    let (lat, lon) = s.split_once(',').ok_or("expected LAT,LON")?;
    let lat: f64 = lat.trim().parse().map_err(|_| "bad latitude")?;
    let lon: f64 = lon.trim().parse().map_err(|_| "bad longitude")?;
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return Err("out of range".into());
    }
    Ok(LatLon::new(lat, lon))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("taksim=info")))
        .with_writer(std::io::stderr)
        .init();

    let cancel = CancellationToken::new();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        }
    });

    match Cli::parse().command {
        Command::Run(args) => run_units(args, cancel).await,
        Command::Listen(args) => {
            let iface = args.group.iface.unwrap_or(Ipv4Addr::UNSPECIFIED);
            info!(group = %args.group.mesh_group, "listening");
            taksim::listen::listen(args.group.mesh_group, iface, args.raw, cancel).await?;
            Ok(())
        }
    }
}

async fn run_units(args: RunArgs, cancel: CancellationToken) -> Result<()> {
    if !args.mesh && args.tcp.is_none() && !args.stdout {
        bail!("nothing to send to: pass at least one of --mesh, --tcp, --stdout");
    }

    let (units, scenario_interval) = match (args.swarm, &args.scenario) {
        (Some(n), _) => {
            if n == 0 || !(args.radius.is_finite() && args.radius > 0.0) {
                bail!("--swarm needs a count above 0 and a positive --radius");
            }
            (swarm(n, args.center, args.radius), Duration::from_secs(5))
        }
        (None, path) => {
            let text = match path {
                Some(p) => std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
                None => taksim::DEMO_SCENARIO.to_owned(),
            };
            let scenario = Scenario::parse(&text).context("invalid scenario")?;
            (scenario.build_units()?, scenario.interval())
        }
    };
    let interval = match args.interval {
        Some(s) if s.is_finite() && s >= 0.1 => Duration::from_secs_f64(s),
        Some(_) => bail!("--interval must be at least 0.1"),
        None => scenario_interval,
    };

    let mesh = args
        .mesh
        .then(|| Mesh::new(args.group.mesh_group, args.group.iface, args.ttl, args.mesh_format))
        .transpose()
        .context("opening multicast socket")?;

    if let Some(secs) = args.duration {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(secs.max(0.0))).await;
            cancel.cancel();
        });
    }

    info!(units = units.len(), ?interval, mesh = args.mesh, tcp = ?args.tcp, "starting");
    let outputs = Outputs {
        mesh: mesh.map(Arc::new),
        tcp: args.tcp.map(|addr| (addr, args.tcp_format)),
        stdout: args.stdout,
    };
    run(units, interval, outputs, cancel).await;
    info!("stopped");
    Ok(())
}
