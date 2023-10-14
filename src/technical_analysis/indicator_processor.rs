//! Actor which manage yata indicator
//!
//! Dimension

use async_trait::async_trait;
use chrono::Utc;
use futures::lock::Mutex;
use opentelemetry::metrics::Meter;
use serde::Deserialize;
use std::{collections::HashMap, fmt::Debug, sync::Arc, vec};
use tokio::{
    sync::broadcast,
    task::{self, JoinHandle},
};
use uuid::Uuid;

use super::{
    candle::Candle,
    event::{MarketEvent, MarketProduct},
    product::{Indicator, ProductInstance},
};
use crate::{
    actor::Actor,
    channel::{Post, PostChannel, PostFilter},
    error::Error,
    processor::{launch_processor, Processor, ProcessorConfig, ProcessorMetrics},
};

const DEFAULT_INDICATOR_CONCURRENCY: usize = 500;
const DEFAULT_BATCH_SIZE: usize = 100;

/// An indicator proxy subscribe to a channel
pub struct IndicatorProcessor {
    _config: IndicatorProcessorConfig,
    products: Mutex<Vec<ProductInstance>>,
    meter: Arc<Meter>,
    uuid: Uuid,
    metrics: ProcessorMetrics,
}

impl IndicatorProcessor {
    fn new(config: IndicatorProcessorConfig, meter: Arc<Meter>, uuid: Uuid) -> Self {
        let metrics = ProcessorMetrics::new(config.name.clone(), meter.clone());

        Self {
            _config: config,
            products: Mutex::new(Vec::new()),
            uuid,
            meter,
            metrics,
        }
    }
}

impl Actor for IndicatorProcessor {
    fn name(&self) -> String {
        self._config.name.to_string()
    }

    fn uuid(&self) -> Uuid {
        self.uuid
    }
}

#[async_trait]
impl Processor for IndicatorProcessor {
    /// Flush all indicators
    async fn flush(&self, _: &Post) -> Result<Vec<Post>, Error> {
        Ok(vec![])
    }

    /// Reset indicators
    async fn reset(&self) -> Result<Vec<Post>, Error> {
        let mut products_guard = self.products.lock().await;
        for product in products_guard.iter_mut() {
            (*product).reset();
        }

        Ok(vec![])
    }

    /// Dispatch input according to the product
    async fn write(&self, post: &Post) -> Result<Vec<Post>, Error> {
        log::debug!("Received indicator message: {:?}", post.body);

        let mut products_guard = self.products.lock().await;

        let candles = serde_json::from_value::<Vec<Candle>>(post.body.clone())?;
        let mut events = vec![];

        for candle in candles.iter() {
            let result = if let Some(product) = products_guard.iter_mut().find(|p| **p == *candle) {
                (*product).next(candle)?
            } else {
                let mut product =
                    ProductInstance::try_new(candle, &self._config.indicator, self.meter.clone())?;

                let result = product.next(candle)?;
                products_guard.push(product);
                result
            };

            if let Some(result) = result {
                events.push(MarketEvent {
                    id: Uuid::new_v4(),
                    result,
                    product: MarketProduct {
                        source: candle.source.to_string(),
                        channel: candle.channel.to_string(),
                        indicator: self.name(),
                        product: candle.product.to_string(),
                        period: candle.period,
                    },
                    datetime: Utc::now(),
                });
            };
        }

        events
            .chunks(self._config.batch_size)
            .map(|x: &[MarketEvent]| -> Result<Post, Error> {
                let tag = format!("indicator_{}", self.name());

                Ok(Post::new(
                    serde_json::to_value(x.to_vec())?,
                    HashMap::new(),
                    tag,
                ))
            })
            .collect()
    }

    fn metrics(&self) -> ProcessorMetrics {
        self.metrics.clone()
    }

    fn config(&self) -> Box<dyn ProcessorConfig + Send + Sync> {
        Box::new(self._config.clone())
    }
}

fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}

fn default_concurrency() -> usize {
    DEFAULT_INDICATOR_CONCURRENCY
}

fn default_channel() -> PostChannel {
    PostChannel::Signal
}

#[derive(Clone, Debug, Deserialize)]
pub struct IndicatorProcessorConfig {
    #[serde(rename = "concurrency", default = "default_concurrency")]
    pub _concurrency: usize,
    #[serde(rename = "input", default = "default_channel")]
    pub _input: PostChannel,
    #[serde(rename = "output", default = "default_channel")]
    pub _output: PostChannel,
    pub name: String,
    pub indicator: Indicator,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    pub filter: PostFilter,
}

#[async_trait]
impl ProcessorConfig for IndicatorProcessorConfig {
    async fn build(
        &self,
        meter: Arc<Meter>,
        order: broadcast::Sender<Post>,
        input: broadcast::Sender<Post>,
        output: broadcast::Sender<Post>,
    ) -> Result<
        (
            Arc<Box<dyn Processor + Send + Sync>>,
            JoinHandle<Result<(), Error>>,
        ),
        Error,
    > {
        log::info!("Creating processor: {:?}", self.name);
        let processor: Arc<Box<dyn Processor + Sync + Send>> = Arc::new(Box::new(
            IndicatorProcessor::new(self.clone(), meter, Uuid::new_v4()),
        ));

        log::info!("Launching processor: {:?}", self.name);

        let expected_match = Box::new(self.filter.clone());

        let filter = move |post: &Post| -> bool {
            let r#match = *expected_match.to_owned();
            r#match.is_match(post)
        };

        let task = task::spawn(launch_processor(
            processor.clone(),
            Box::new(filter),
            self._concurrency,
            order,
            input,
            output,
        ));

        log::info!("Processor launched: {:?}", self.name);

        Ok((processor, task))
    }

    fn concurrency(&self) -> usize {
        self._concurrency
    }

    fn input(&self) -> PostChannel {
        self._input
    }

    fn output(&self) -> PostChannel {
        self._output
    }
}
