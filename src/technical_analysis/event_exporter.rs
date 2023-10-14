//! Dynamodb exporter

use async_trait::async_trait;
use futures::lock::Mutex;
use opentelemetry::metrics::Meter;
use rusoto_core::Region;
use rusoto_dynamodb::{BatchWriteItemInput, DynamoDb, DynamoDbClient, WriteRequest};
use serde::Deserialize;
use std::{collections::HashMap, str::FromStr, sync::Arc, vec::Vec};
use tokio::{
    sync::broadcast,
    task::{self, JoinHandle},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    actor::Actor,
    channel::{launch_timer, Order, Post, PostChannel, PostFilter, Timer},
    error::Error,
    exporter::{launch_exporter, Exporter, ExporterConfig, ExporterMetrics},
};

use super::event::MarketEvent;

/// Maximum dynamodb write batch size
const MAX_BATCH_SIZE: usize = 25;

pub struct MarketEventExporter {
    _config: MarketEventExporterConfig,
    buffer: Mutex<Vec<MarketEvent>>,
    client: DynamoDbClient,
    state: Mutex<Vec<MarketEvent>>,
    table: String,
    uuid: Uuid,
    metrics: ExporterMetrics,
}

impl MarketEventExporter {
    pub fn try_new(
        config: MarketEventExporterConfig,
        client: DynamoDbClient,
        table: String,
        timer: Timer,
        uuid: Uuid,
        meter: Arc<Meter>,
    ) -> Result<Self, Error> {
        task::spawn(launch_timer(timer));
        let metrics = ExporterMetrics::new(config.name.clone(), meter);

        let exporter = Self {
            _config: config,
            buffer: Mutex::new(Vec::new()),
            client,
            state: Mutex::new(Vec::new()),
            table,
            uuid,
            metrics,
        };

        Ok(exporter)
    }
}

#[async_trait]
impl Exporter for MarketEventExporter {
    async fn flush(&self) -> Result<(), Error> {
        let mut buffer_guard = self.buffer.lock().await;
        log::debug!(
            "Exporter {:?} flushing {:?} records..",
            self.name(),
            buffer_guard.len()
        );

        let batchs = buffer_guard
            .iter()
            .map(|event: &MarketEvent| -> Result<WriteRequest, Error> {
                Ok(WriteRequest {
                    put_request: Some(event.clone().try_into()?),
                    delete_request: None,
                })
            })
            .collect::<Result<Vec<WriteRequest>, Error>>()?
            .chunks(MAX_BATCH_SIZE)
            .map(|x| x.to_vec())
            .collect::<Vec<Vec<WriteRequest>>>();

        for batch in batchs {
            self.client
                .batch_write_item(BatchWriteItemInput {
                    request_items: HashMap::from([(self.table.clone(), batch)]),
                    return_consumed_capacity: None,
                    return_item_collection_metrics: None,
                })
                .await?;
        }

        buffer_guard.clear();

        Ok(())
    }

    async fn write(&self, chunk: Vec<Post>) -> Result<(), Error> {
        let mut state_guard = self.state.lock().await;
        let mut buffer_guard = self.buffer.lock().await;
        let mut register = move |event: &MarketEvent| -> Result<(), Error> {
            if let Some(state) = state_guard.iter_mut().find(|s| *s == event) {
                if state.has_changed(event) {
                    *state = event.clone();
                    buffer_guard.push(event.clone());
                };
            } else {
                state_guard.push(event.clone());
            };

            Ok(())
        };

        chunk
            .into_iter()
            .try_for_each(|post: Post| -> Result<(), Error> {
                if let Ok(events) = serde_json::from_value::<Vec<MarketEvent>>(post.body.clone()) {
                    for event in events.iter() {
                        register(event)?;
                    }

                    Ok(())
                } else if let Ok(event) = serde_json::from_value::<MarketEvent>(post.body.clone()) {
                    register(&event)
                } else {
                    Err(Error::UnprocessableEvent(format!(
                        "Failed to parse: {:?}",
                        post
                    )))
                }
            })
    }

    fn metrics(&self) -> ExporterMetrics {
        self.metrics.clone()
    }

    fn config(&self) -> Box<dyn ExporterConfig + Send + Sync> {
        Box::new(self._config.clone())
    }
}

impl Actor for MarketEventExporter {
    fn name(&self) -> String {
        self._config.name.to_string()
    }

    fn uuid(&self) -> Uuid {
        self.uuid
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketConfig {}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketEventExporterConfig {
    #[serde(rename = "chunk_size")]
    pub _chunk_size: usize,
    #[serde(rename = "concurrency")]
    pub _concurrency: usize,
    #[serde(rename = "input")]
    pub _input: PostChannel,
    pub name: String,
    pub filter: PostFilter,
    pub export_period: Duration,
    pub region: String,
    pub endpoint: Option<String>,
    pub table: String,
}

#[async_trait]
impl ExporterConfig for MarketEventExporterConfig {
    fn chunk_size(&self) -> usize {
        self._chunk_size
    }

    fn concurrency(&self) -> usize {
        self._concurrency
    }

    fn input(&self) -> PostChannel {
        self._input
    }

    async fn build(
        &self,
        meter: Arc<Meter>,
        order: broadcast::Sender<Post>,
        input: broadcast::Sender<Post>,
    ) -> Result<
        (
            Arc<Box<dyn Exporter + Send + Sync>>,
            JoinHandle<Result<(), Error>>,
        ),
        Error,
    > {
        log::info!("Creating exporter: {:?}", self.name);
        let uuid = Uuid::new_v4();

        let timer = Timer::new(
            order.clone(),
            Order::Flush { uuid, reason: None },
            vec![self.export_period],
            format!("{}_timer", self.name),
            meter.clone(),
        );

        let region = if let Some(endpoint) = self.endpoint.clone() {
            Region::Custom {
                name: self.region.clone(),
                endpoint,
            }
        } else {
            Region::from_str(self.region.as_str()).unwrap_or(Region::UsEast1)
        };

        let exporter: Arc<Box<dyn Exporter + Send + Sync>> =
            Arc::new(Box::new(MarketEventExporter::try_new(
                self.clone(),
                DynamoDbClient::new(region),
                self.table.clone(),
                timer,
                uuid,
                meter,
            )?));

        log::info!("Launching exporter: {:?}", self.name);

        let expected_match = Box::new(self.filter.clone());

        let filter = move |post: &Post| -> bool {
            let r#match = *expected_match.to_owned();
            r#match.is_match(post)
        };

        let task = task::spawn(launch_exporter(
            exporter.clone(),
            Box::new(filter),
            order,
            input,
        ));

        log::info!("Exporter launched: {:?}", self.name);

        Ok((exporter, task))
    }
}
