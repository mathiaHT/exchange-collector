use core::pin::Pin;
use enum_dispatch::enum_dispatch;
use futures::future::{join_all, JoinAll};
use opentelemetry::metrics::Meter;
use regex::Regex;
use rusoto_core::Region;
use rusoto_s3::{GetObjectRequest, S3Client, S3};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Cursor, Read},
    path::PathBuf,
    sync::Arc,
};
use tokio::{io::AsyncReadExt, sync::broadcast, task::JoinHandle, time::Duration};
use tokio_tungstenite::tungstenite::Message as TMessage;

use crate::{
    channel::{Post, PostChannel},
    delta::DeltaExporterConfig,
    error::Error,
    exchange::ExchangeReceiverConfig,
    exporter::{Exporter, ExporterConfig},
    metrics::MetricsConfig,
    processor::{Processor, ProcessorConfig},
    technical_analysis::{
        CandleProcessorConfig, IndicatorProcessorConfig, MarketEventExporterConfig,
    },
    websocket::{WebsocketClient, WebsocketReceiverConfig, WebsocketSource},
};

const DEFAULT_CHANNEL_CAPACITY: usize = 100;
const DEFAULT_EXPORTER_GRACEFUL_PERIOD: u64 = 15;

pub struct Channel<T> {
    #[allow(dead_code)]
    receiver: broadcast::Receiver<T>,
    sender: broadcast::Sender<T>,
}

impl<T: Clone> Channel<T> {
    fn new(capacity: usize) -> Self {
        let (sender, receiver) = broadcast::channel::<T>(capacity);

        Self { receiver, sender }
    }
}

#[allow(dead_code)]
pub struct Pipeline {
    config: PipelineConfig,

    receivers: Vec<Arc<Box<dyn WebsocketClient + Send + Sync>>>,
    processors: Vec<Arc<Box<dyn Processor + Send + Sync>>>,
    exporters: Vec<Arc<Box<dyn Exporter + Send + Sync>>>,
    channels: HashMap<PostChannel, Channel<Post>>,
}

/// Represents an subscription to send to a websocket server
#[enum_dispatch(WebsocketReceiverConfig)]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PipelineReceiverConfig {
    #[serde(rename = "exchange")]
    Exchange(ExchangeReceiverConfig),
}

#[enum_dispatch(ExporterConfig)]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PipelineExporterConfig {
    #[serde(rename = "delta")]
    Delta(DeltaExporterConfig),
    #[serde(rename = "market_event")]
    MarketEvent(MarketEventExporterConfig),
}

#[enum_dispatch(ProcessorConfig)]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PipelineProcessorConfig {
    #[serde(rename = "candle")]
    Candle(CandleProcessorConfig),
    #[serde(rename = "indicator")]
    Indicator(IndicatorProcessorConfig),
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChannelConfig {
    pub signal_capacity: usize,
    pub data_capacity: usize,
    pub order_capacity: usize,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            signal_capacity: DEFAULT_CHANNEL_CAPACITY,
            data_capacity: 5 * DEFAULT_CHANNEL_CAPACITY,
            order_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceConfig {
    pub graceful_period: Duration,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub channels: ChannelConfig,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            graceful_period: Duration::from_secs(DEFAULT_EXPORTER_GRACEFUL_PERIOD),
            metrics: MetricsConfig::default(),
            channels: ChannelConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PipelineConfig {
    #[serde(default)]
    pub service: ServiceConfig,
    #[serde(default)]
    pub exporters: Vec<PipelineExporterConfig>,
    #[serde(default)]
    pub processors: Vec<PipelineProcessorConfig>,
    #[serde(default)]
    pub receivers: Vec<PipelineReceiverConfig>,
}

impl PipelineConfig {
    pub async fn try_from(path: PathBuf, region: Region) -> Result<Self, Error> {
        let re = Regex::new(r"s3://(?P<bucket>[^/]*)/(?P<key>.*)").unwrap();

        log::info!("use path {}", path.to_str().unwrap_or(""));

        let reader: Box<dyn Read> = if let Some(captures) = re.captures(path.to_str().unwrap_or(""))
        {
            log::info!("Retrieve config from s3.");

            let request = GetObjectRequest {
                bucket: captures.name("bucket").unwrap().as_str().to_string(),
                key: captures.name("key").unwrap().as_str().to_string(),
                ..Default::default()
            };
            let mut stream = S3Client::new(region)
                .get_object(request)
                .await?
                .body
                .ok_or_else(|| Error::Config(path.to_str().unwrap_or("Invalid path.").to_string()))?
                .into_async_read();

            let mut buffer = Vec::new();

            stream.read_to_end(&mut buffer).await?;

            Box::new(Cursor::new(buffer))
        } else {
            log::info!("Retrieve config from local file.");

            let file = File::open(path.clone())?;
            Box::new(BufReader::new(file))
        };

        let config: PipelineConfig = match path.extension() {
            None => Err(Error::Config(
                path.to_str().unwrap_or("Invalid path.").to_string(),
            )),
            Some(os_str) => match os_str.to_str() {
                Some("json") => serde_json::from_reader(reader).map_err(Error::from),
                Some("yaml") | Some("yml") => serde_yaml::from_reader(reader).map_err(Error::from),
                _ => Err(Error::Config("Invalid extension.".to_string())),
            },
        }?;

        Ok(config)
    }

    pub async fn init(
        self,
        meter: Arc<Meter>,
    ) -> Result<(Pipeline, JoinAll<JoinHandle<Result<(), Error>>>), Error> {
        let mut channels: HashMap<PostChannel, Channel<Post>> = HashMap::new();

        channels.insert(
            PostChannel::Data,
            Channel::<Post>::new(self.service.channels.signal_capacity),
        );
        channels.insert(
            PostChannel::Signal,
            Channel::<Post>::new(self.service.channels.data_capacity),
        );
        channels.insert(
            PostChannel::Order,
            Channel::<Post>::new(self.service.channels.order_capacity),
        );

        let mut handles: Vec<JoinHandle<Result<(), Error>>> = vec![];
        let mut receivers: Vec<Arc<Box<dyn WebsocketClient + Send + Sync>>> = vec![];
        let mut processors: Vec<Arc<Box<dyn Processor + Send + Sync>>> = vec![];
        let mut exporters: Vec<Arc<Box<dyn Exporter + Send + Sync>>> = vec![];

        for config in self.exporters.iter() {
            let (exporter, handle) = config
                .build(
                    meter.clone(),
                    channels
                        .get(&PostChannel::Order)
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                    channels
                        .get(&config.input())
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                )
                .await?;

            exporters.push(exporter);
            handles.push(handle);
        }

        log::info!("Exporters created.");

        for config in self.processors.iter() {
            let (processor, handle) = config
                .build(
                    meter.clone(),
                    channels
                        .get(&PostChannel::Order)
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                    channels
                        .get(&config.input())
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                    channels
                        .get(&config.output())
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                )
                .await?;

            processors.push(processor);
            handles.push(handle);
        }

        log::info!("Processors created");

        for config in self.receivers.iter() {
            let (receiver, handle) = config
                .build(
                    meter.clone(),
                    channels
                        .get(&PostChannel::Order)
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                    channels
                        .get(&config.output())
                        .ok_or(Error::Setup)?
                        .sender
                        .clone(),
                )
                .await?;

            receivers.push(receiver);
            handles.push(handle);
        }

        log::info!("Receivers created");

        let config = self;
        let pipeline = Pipeline {
            config,
            exporters,
            receivers,
            processors,
            channels,
        };

        log::info!("Pipeline created");

        Ok((pipeline, join_all(handles)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchange::ExchangeSpecificConfig;
    use rstest::*;

    #[fixture]
    fn configuration_data() -> &'static str {
        r#"
        {
            "service": {
                "graceful_period": {
                    "secs": 30,
                    "nanos": 0
                },
                "metrics": {
                    "type": "otlp",
                    "period": {
                        "secs": 15,
                        "nanos": 0
                    }
                }
            },
            "receivers": [
                {
                    "name": "coinbase_receiver",
                    "type": "exchange",
                    "exchange": {
                        "uri": "wss://coinbase.com",
                        "source": "coinbase",
                        "subscription": {
                            "type": "subscribe",
                            "product_ids": [
                                "ETH-USD"
                            ],
                            "channels": [
                                "matches"
                            ]
                        }
                    },
                    "concurrency": 3,
                    "output": "signal"
                }
            ],
            "processors": [
                {
                    "name": "candle",
                    "type": "candle",
                    "periods": [
                        {
                            "secs": 60,
                            "nanos": 0
                        },
                        {
                            "secs": 300,
                            "nanos": 0
                        }
                    ],
                    "filter": {
                        "tag_pattern": "websocket_*"
                    },
                    "concurrency": 5,
                    "input": "data",
                    "output": "signal"
                },
                {
                    "name": "macd",
                    "type": "indicator",
                    "batch_size": 10,
                    "indicator": {
                        "type":"macd",
                        "ma1": {
                            "ema": 12
                        },
                        "ma2": {
                            "ema": 26
                        },
                        "signal": {
                            "ema": 9
                        },
                        "source": "close"
                    },
                    "filter": {
                        "tag_pattern": "candle"
                    },
                    "concurrency": 2,
                    "input": "data",
                    "output": "signal"
                }
            ],
            "exporters": [
                {
                    "name": "coinbase_match",
                    "type": "delta",
                    "provider": "aws",
                    "table": {
                        "database": "coinbase",
                        "table": "match"
                    },
                    "period": {
                        "secs": 60,
                        "nanos": 0
                    },
                    "filter": {
                        "tag_pattern": "coinbase_*"
                    },
                    "concurrency": 2,
                    "chunk_size": 1,
                    "input": "data",
                    "region": "eu-west-1",
                    "bucket": "datalake",
                    "prefix": "data",
                    "lock_table": "datalake-lock",
                    "channels": [
                        "matches"
                    ]
                },
                {
                    "name": "market_event",
                    "type": "market_event",
                    "export_period": {
                        "secs": 60,
                        "nanos": 0
                    },
                    "filter": {
                        "tag_pattern": "indicator.*"
                    },
                    "concurrency": 2,
                    "chunk_size": 1,
                    "region": "eu-west-1",
                    "table": "market-event",
                    "input": "data"
                }
            ]
        }
        "#
    }

    #[rstest]
    fn test_configuration(configuration_data: &str) {
        let configuration = serde_json::from_str::<PipelineConfig>(configuration_data).unwrap();

        for config in configuration.receivers {
            match &config {
                PipelineReceiverConfig::Exchange(config) => {
                    matches!(config.exchange, ExchangeSpecificConfig::Coinbase { .. });
                }
            }
        }
    }
}
