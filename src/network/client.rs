use std::{collections::HashMap, sync::Arc};

use contract_client::PeerId;
use futures::{Stream, StreamExt};
use parking_lot::{Mutex, RwLock};
use subsquid_messages::{data_chunk::DataChunk, query_result, Ping, Query, QueryResult};
use subsquid_network_transport::{
    GatewayConfig, GatewayEvent, GatewayTransportHandle, P2PTransportBuilder, QueueFull,
    TransportArgs,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    cli::Config,
    types::{generate_query_id, DatasetId, QueryId},
    utils::UseOnce,
};

use super::{NetworkState, StorageClient};

/// Tracks the network state and handles p2p communication
pub struct NetworkClient {
    incoming_events: UseOnce<Box<dyn Stream<Item = GatewayEvent> + Send + Unpin + 'static>>,
    transport_handle: GatewayTransportHandle,
    network_state: RwLock<NetworkState>,
    tasks: Mutex<HashMap<QueryId, QueryTask>>,
    dataset_storage: StorageClient,
}

struct QueryTask {
    result_tx: oneshot::Sender<query_result::Result>,
    worker_id: PeerId,
}

impl NetworkClient {
    pub async fn new(
        args: TransportArgs,
        logs_collector: PeerId,
        config: Arc<Config>,
    ) -> anyhow::Result<NetworkClient> {
        tracing::info!("Listing existing chunks");
        let dataset_storage = StorageClient::new(config.available_datasets.values()).await?;
        let transport_builder = P2PTransportBuilder::from_cli(args).await?;
        let mut gateway_config = GatewayConfig::new(logs_collector);
        gateway_config.query_config.send_timeout = config.transport_timeout;
        let (incoming_events, transport_handle) =
            transport_builder.build_gateway(gateway_config)?;
        Ok(NetworkClient {
            transport_handle,
            incoming_events: UseOnce::new(Box::new(incoming_events)),
            network_state: RwLock::new(NetworkState::new(config)),
            tasks: Mutex::new(HashMap::new()),
            dataset_storage,
        })
    }

    pub async fn run(&self, cancellation_token: CancellationToken) {
        let stream = self
            .incoming_events
            .take()
            .unwrap()
            .take_until(cancellation_token.cancelled_owned());
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            match event {
                GatewayEvent::Ping { peer_id, ping } => {
                    self.handle_ping(peer_id, ping);
                }
                GatewayEvent::QueryResult { peer_id, result } => {
                    self.handle_query_result(peer_id, result)
                        .unwrap_or_else(|e| {
                            tracing::error!("Error handling query: {e:?}");
                        });
                }
            }
        }
    }

    pub fn find_chunk(&self, dataset: &DatasetId, block: u64) -> Option<DataChunk> {
        self.dataset_storage.find_chunk(dataset, block)
    }

    pub fn next_chunk(&self, dataset: &DatasetId, chunk: &DataChunk) -> Option<DataChunk> {
        self.dataset_storage.next_chunk(dataset, chunk)
    }

    pub fn find_worker(&self, dataset: &DatasetId, chunk: &DataChunk) -> Option<PeerId> {
        self.network_state
            .read()
            .find_worker(dataset, chunk.first_block())
    }

    pub fn get_height(&self, dataset: &DatasetId) -> Option<u64> {
        self.network_state.read().get_height(dataset)
    }

    pub fn query_worker(
        &self,
        worker: &PeerId,
        dataset: &DatasetId,
        query: String,
    ) -> Result<oneshot::Receiver<query_result::Result>, QueueFull> {
        let query_id = generate_query_id();

        self.transport_handle.send_query(
            *worker,
            Query {
                dataset: Some(dataset.to_string()),
                query_id: Some(query_id.clone()),
                query: Some(query),
                client_state_json: Some("{}".to_string()), // This is a placeholder field
                ..Default::default()
            },
        )?;
        tracing::trace!("Sent query {query_id} to {worker}");

        let (result_tx, result_rx) = oneshot::channel();
        let task = QueryTask {
            result_tx,
            worker_id: *worker,
        };
        self.tasks.lock().insert(query_id, task);
        Ok(result_rx)
    }

    fn handle_ping(&self, peer_id: PeerId, ping: Ping) {
        tracing::trace!("Ping from {peer_id}");
        let worker_state = ping
            .stored_ranges
            .into_iter()
            .map(|r| (DatasetId::from_url(r.url), r.ranges.into()))
            .collect();
        self.network_state
            .write()
            .update_dataset_states(peer_id, worker_state);
    }

    fn handle_query_result(&self, peer_id: PeerId, result: QueryResult) -> anyhow::Result<()> {
        let QueryResult { query_id, result } = result;
        let result = result.ok_or_else(|| anyhow::anyhow!("Result missing"))?;
        tracing::trace!("Got result for query {query_id}");

        let (query_id, task) = self
            .tasks
            .lock()
            .remove_entry(&query_id)
            .ok_or_else(|| anyhow::anyhow!("Not expecting response for query {query_id}"))?;
        if peer_id != task.worker_id {
            tracing::error!(
                "Invalid message sender, expected {}, got {}",
                task.worker_id,
                peer_id
            );
            self.network_state.write().greylist_worker(peer_id);
        }

        match &result {
            // Greylist worker if server error occurred during query execution
            query_result::Result::ServerError(e) => {
                tracing::warn!("Server error returned for query {query_id}: {e}");
                self.network_state.write().greylist_worker(peer_id);
            }
            // Add worker to the missing allocations cache
            query_result::Result::NoAllocation(()) => {
                self.network_state.write().no_allocation_for_worker(peer_id);
            }
            _ => {}
        }

        task.result_tx.send(result).ok();

        Ok(())
    }
}
