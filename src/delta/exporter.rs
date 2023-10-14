//! Deltalake exporter
//!
//! Delta Lake is a storage layer that brings scalable, ACID transactions.. Through
//! the DeltaTable and BufferedJsonWriter, the library allows to stream data to
//! aws, azure and gcp.

use crate::{
    actor::Actor,
    channel::{launch_timer, Order, Post, PostChannel, PostFilter, Timer},
    error::Error,
    exporter::{launch_exporter, Exporter, ExporterConfig, ExporterMetrics},
    matches::{build_matches_batch, MatchTable},
    metrics::metadata::CHANNEL,
    record::{PriceVolume, Record},
};
use async_trait::async_trait;
use deltalake::writer::{record_batch::RecordBatchWriter, DeltaWriter};
use futures::lock::Mutex;
use opentelemetry::metrics::Meter;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr, string::ToString, sync::Arc};
use tokio::{
    sync::broadcast,
    task::{self, JoinHandle},
    time::Duration,
};
use uuid::Uuid;

/// Export structured data using a delta table as sink
pub struct DeltaExporter {
    _config: DeltaExporterConfig,
    _buffers: Mutex<HashMap<String, RecordBatchWriter>>,
    _uuid: Uuid,
    _metrics: ExporterMetrics,
}

impl DeltaExporter {
    pub fn try_new(
        config: DeltaExporterConfig,
        buffers: Mutex<HashMap<String, RecordBatchWriter>>,
        timer: Timer,
        uuid: Uuid,
        meter: Arc<Meter>,
    ) -> Result<Self, Error> {
        task::spawn(launch_timer(timer));

        let metrics = ExporterMetrics::new(config.name.clone(), meter);

        let exporter = Self {
            _config: config,
            _uuid: uuid,
            _buffers: buffers,
            _metrics: metrics,
        };

        Ok(exporter)
    }
}

#[async_trait]
impl Exporter for DeltaExporter {
    async fn flush(&self) -> Result<(), Error> {
        let mut buffers_guard = self._buffers.lock().await;

        for (name, buffer) in buffers_guard.iter_mut() {
            log::info!(
                "Exporter {:?} flushing {:?} records..",
                name,
                buffer.buffered_record_batch_count()
            );
            buffer.flush().await?;
            log::debug!("Exporter flushed.");
        }
        Ok(())
    }

    async fn write(&self, chunk: Vec<Post>) -> Result<(), Error> {
        let records = chunk
            .into_iter()
            .map(|post: Post| -> Result<Record, Error> {
                Ok(serde_json::from_value::<Record>(post.body)?)
            })
            .collect::<Result<Vec<Record>, Error>>()?;

        // simulate a group by channel
        // itertools.groupby does not work in async code
        // https://users.rust-lang.org/t/problem-sharing-futures-across-threads-when-call-from-warp-filter/46830/3
        let mut batchs: HashMap<String, Vec<Record>> = HashMap::new();
        for record in records.into_iter() {
            if let Some((_, batch)) = batchs.iter_mut().find(|(k, _)| **k == record.channel()) {
                batch.push(record)
            } else {
                batchs.insert(record.channel(), vec![record]);
            };
        }

        let mut buffers_guard = self._buffers.lock().await;
        for (channel, group) in batchs.into_iter() {
            let batch = match ExportChannels::from_str(channel.as_str())? {
                ExportChannels::Matches => build_matches_batch(group)?,
            };

            if let Some((_, buffer)) = buffers_guard.iter_mut().find(|(k, _)| **k == channel) {
                buffer.write_partition(batch, &HashMap::new()).await?;
            };
        }

        Ok(())
    }

    fn metrics(&self) -> ExporterMetrics {
        self._metrics.clone()
    }

    fn config(&self) -> Box<dyn ExporterConfig + Send + Sync> {
        Box::new(self._config.clone())
    }
}

impl Actor for DeltaExporter {
    fn name(&self) -> String {
        self._config.name.to_string()
    }

    fn uuid(&self) -> Uuid {
        self._uuid
    }
}

#[derive(
    strum_macros::Display, strum_macros::EnumString, Copy, Clone, Debug, Deserialize, Serialize,
)]
pub enum ExportChannels {
    #[serde(rename = "matches")]
    #[strum(serialize = "matches", serialize = "match")]
    Matches,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeltaExporterConfig {
    #[serde(default, rename = "chunk_size")]
    pub _chunk_size: usize,
    #[serde(rename = "concurrency")]
    pub _concurrency: usize,
    #[serde(rename = "input")]
    pub _input: PostChannel,
    pub name: String,
    pub filter: PostFilter,
    pub region: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub period: Duration,
    pub lock_table: String,
    pub bucket: String,
    pub prefix: String,
    pub channels: Vec<ExportChannels>,
}

#[async_trait]
impl ExporterConfig for DeltaExporterConfig {
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

        // Initialize flush timer
        let timer = Timer::new(
            order.clone(),
            Order::Flush { uuid, reason: None },
            vec![self.period],
            format!("{}_timer", self.name),
            meter.clone(),
        );

        let mut buffers: HashMap<String, RecordBatchWriter> = HashMap::new();
        for channel in self.channels.iter() {
            let table = match channel {
                ExportChannels::Matches => {
                    MatchTable::get_or_create(
                        self.bucket.clone(),
                        self.prefix.clone(),
                        self.lock_table.clone(),
                        self.region.clone(),
                        self.endpoint.clone(),
                    )
                    .await?
                }
            };
            let buffer = RecordBatchWriter::for_table(&table)?;
            buffers.insert(channel.to_string(), buffer);
        }

        let exporter: Arc<Box<dyn Exporter + Send + Sync>> =
            Arc::new(Box::new(DeltaExporter::try_new(
                self.clone(),
                Mutex::new(buffers),
                timer,
                uuid,
                meter.clone(),
            )?));

        let mut filter = self.filter.to_owned();
        filter.body_attributes.insert(
            CHANNEL.to_string(),
            self.channels
                .clone()
                .into_iter()
                .map(|c| c.to_string())
                .collect(),
        );
        let expected_match = Box::new(filter);

        let filter = move |post: &Post| -> bool {
            let r#match = *expected_match.to_owned();
            r#match.is_match(post)
        };

        log::info!("Launching exporter: {:?}", self.name);
        let task = task::spawn(launch_exporter(
            exporter.clone(),
            Box::new(filter),
            order.clone(),
            input.clone(),
        ));

        log::info!("Exporter launched: {:?}", self.name);

        Ok((exporter, task))
    }
}
