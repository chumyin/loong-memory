use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use loong_memoryd::{serve_with_shutdown, ServiceConfig, ServiceState};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "loong-memoryd")]
#[command(about = "Minimal loong-memory HTTP daemon", version)]
struct Cli {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long, default_value = "127.0.0.1:3000")]
    listen_addr: SocketAddr,
    #[arg(long)]
    policy_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cli = Cli::parse();
    let config = ServiceConfig::new(cli.db.clone(), cli.policy_file.clone());
    let state = ServiceState::from_config(&config).context("build loong-memoryd state")?;
    let listener = TcpListener::bind(cli.listen_addr)
        .await
        .with_context(|| format!("bind loong-memoryd listener {}", cli.listen_addr))?;
    let address = listener
        .local_addr()
        .context("resolve loong-memoryd bound address")?;

    info!(
        address = %address,
        db = %cli.db.display(),
        "starting loong-memoryd"
    );

    serve_with_shutdown(listener, state, async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %err, "failed to listen for ctrl-c");
        }
    })
    .await
}
