use std::borrow::Cow;
use std::sync::Arc;

use clap::Parser;
use cli::Cli;
use controller::task_manager::TaskManager;
use http_server::run_server;
use network::NetworkClient;
use prometheus_client::registry::Registry;
use tokio_util::sync::CancellationToken;

mod cli;
mod controller;
mod http_server;
mod metrics;
mod network;
mod types;
mod utils;

fn setup_tracing(json: bool) -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::builder().parse_lossy(
        std::env::var(tracing_subscriber::EnvFilter::DEFAULT_ENV)
            .unwrap_or(format!("info,{}=debug", std::env!("CARGO_CRATE_NAME"))),
    );

    if json {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .json()
            .with_span_list(false)
            .flatten_event(true)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .compact()
            .init();
    };
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    let mut args = Cli::parse();
    setup_tracing(args.json_log)?;

    let dataset_info = network::datasets_info_load().await?;
    let hostname = args.config.hostname.clone();
    args.config
        .available_datasets
        .iter_mut()
        .for_each(move |(path, d)| {
            if let Some(info) = dataset_info.get_by_path(&path) {
                let provider = info.providers.first().cloned().unwrap_or_default();

                d.chain_id = info.chain_id;
                d.chain_ss58_prefix = info.chain_ss58_prefix;
                d.chain_type = info.chain_type.clone();
                d.name = info.chain_name.clone();
                d.is_testnet = info.is_testnet;
                d.data = Option::from(provider.data.clone());
                d.url = Option::from(format!("{}/datasets/{}", hostname, path));
                d.tier = Option::from(provider.support_tier.unwrap_or_default().to_string());
            }
        });

    let config = Arc::new(args.config);
    let network_client =
        Arc::new(NetworkClient::new(args.transport, args.logs_collector_id, config.clone()).await?);

    let mut metrics_registry = Registry::with_labels(
        vec![(
            Cow::Borrowed("portal_id"),
            Cow::Owned(network_client.peer_id().to_string()),
        )]
        .into_iter(),
    );
    metrics::register_metrics(&mut metrics_registry);
    sqd_network_transport::metrics::register_metrics(&mut metrics_registry);
    let cancellation_token = CancellationToken::new();

    tracing::info!("Network client initialized");
    let task_manager = Arc::new(TaskManager::new(
        network_client.clone(),
        config.max_parallel_streams,
    ));

    let (res, ()) = tokio::join!(
        run_server(
            task_manager,
            network_client.clone(),
            metrics_registry,
            &args.http_listen,
            config
        ),
        network_client.run(cancellation_token),
    );
    res?;

    Ok(())
}
