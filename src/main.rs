use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use env_logger::Env;

use subsquid_network_transport::TransportArgs;
use subsquid_network_transport::{GatewayConfig, P2PTransportBuilder};
use tokio::sync::RwLock;

use crate::config::Config;
use crate::network_state::NetworkState;

mod allocations;
mod chain_updates;
mod client;
mod config;
mod http_server;
mod metrics;
mod network_state;
mod query;
mod scheme_extractor;
mod server;
mod task;

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(flatten)]
    pub transport: TransportArgs,

    #[arg(
        long,
        env = "HTTP_LISTEN_ADDR",
        help = "HTTP server listen addr",
        default_value = "0.0.0.0:8000"
    )]
    http_listen: SocketAddr,

    #[arg(long, env, help = "Path to config file", default_value = "config.yml")]
    config_path: PathBuf,

    #[arg(
        long,
        env,
        help = "Path to allocations database file",
        default_value = "allocations.db"
    )]
    allocations_db_path: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Init logger and parse arguments and config
    dotenv::dotenv().ok();
    env_logger::Builder::from_env(Env::default().default_filter_or("info, ethers_providers=warn"))
        .init();
    let args: Cli = Cli::parse();
    Config::read(&args.config_path).await?;

    // Build P2P transport
    let transport_builder = P2PTransportBuilder::from_cli(args.transport).await?;
    let contract_client = transport_builder.contract_client();
    let local_peer_id = transport_builder.local_peer_id();
    let mut gateway_config = GatewayConfig::new(Config::get().logs_collector_id);
    gateway_config.query_config = Config::get().query_config;
    let (incoming_messages, transport_handle) =
        transport_builder.build_gateway(gateway_config)?;

    // Instantiate contract client and check RPC connection
    anyhow::ensure!(
        contract_client.is_gateway_registered(local_peer_id).await?,
        "Client not registered on chain"
    );

    // Initialize allocated/spent CU metrics with zeros
    let workers = contract_client.active_workers().await?;
    metrics::init_workers(workers.iter().map(|w| w.peer_id.to_string()));
    let network_state = Arc::new(RwLock::new(NetworkState::new(workers)));

    // Start query client
    let query_client = client::get_client(
        local_peer_id,
        incoming_messages,
        transport_handle,
        contract_client,
        network_state.clone(),
        args.allocations_db_path,
    )
    .await?;

    // Start HTTP server
    http_server::run_server(query_client, network_state, &args.http_listen).await
}
