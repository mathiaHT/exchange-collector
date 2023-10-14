//! Actor which manage yata indicator
//!
//! Dimension

use async_trait::async_trait;
use futures::lock::Mutex;
use opentelemetry::metrics::Meter;
use serde::Deserialize;
use std::{collections::HashMap, fmt::Debug, sync::Arc};
use tokio::{
    sync::broadcast,
    task::{self, JoinHandle},
    time::Duration,
};
use uuid::Uuid;

use super::candle::{Candle, CandleMetrics};
use crate::{
    actor::Actor,
    channel::{launch_timer, Order, OrderReason, Post, PostChannel, PostFilter, Timer},
    error::Error,
    processor::{launch_processor, Processor, ProcessorConfig, ProcessorMetrics},
    record::Record,
};

const DEFAULT_BATCH_SIZE: usize = 10;

/// An indicator proxy subscribe to a channel
pub struct CandleProcessor {
    _config: CandleProcessorConfig,
    candles: Mutex<Vec<Candle>>,
    uuid: Uuid,
    metrics: ProcessorMetrics,
    candle_metrics: CandleMetrics,
}

impl CandleProcessor {
    fn try_new(
        config: CandleProcessorConfig,
        meter: Arc<Meter>,
        timer: Timer,
        uuid: Uuid,
    ) -> Result<Self, Error> {
        task::spawn(launch_timer(timer));

        let metrics = ProcessorMetrics::new(config.name.clone(), meter.clone());
        let candle_metrics = CandleMetrics::try_new(meter)?;

        Ok(Self {
            _config: config,
            candles: Mutex::new(vec![]),
            metrics,
            uuid,
            candle_metrics,
        })
    }
}

impl Actor for CandleProcessor {
    fn name(&self) -> String {
        self._config.name.to_string()
    }

    fn uuid(&self) -> Uuid {
        self.uuid
    }
}

#[async_trait]
impl Processor for CandleProcessor {
    async fn flush(&self, post: &Post) -> Result<Vec<Post>, Error> {
        if let Ok(Order::Flush {
            reason: Some(OrderReason::Period(period)),
            ..
        }) = post.clone().try_into()
        {
            self.candles
                .lock()
                .await
                .iter_mut()
                .filter_map(|candle: &mut Candle| -> Option<Candle> {
                    if candle.period.eq(&period) {
                        let previous = (*candle).reset();
                        (!previous.is_nan()).then(|| {
                            self.candle_metrics.record(&previous);
                            previous
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<Candle>>()
                .chunks(self._config.batch_size)
                .map(|chunk: &[Candle]| -> Result<Post, Error> {
                    let tag = format!("candle_{}", period.as_secs());

                    Ok(Post::new(
                        serde_json::to_value(chunk.to_vec())?,
                        HashMap::new(),
                        tag,
                    ))
                })
                .collect()
        } else {
            Ok(vec![])
        }
    }

    async fn reset(&self) -> Result<Vec<Post>, Error> {
        let mut candles_guard = self.candles.lock().await;
        for candle in candles_guard.iter_mut() {
            (*candle).reset();
        }

        Ok(vec![])
    }

    async fn write(&self, post: &Post) -> Result<Vec<Post>, Error> {
        log::debug!("Received indicator message: {:?}", post.body);

        let record = Record::try_from(post.clone())?;
        let mut candles_guard = self.candles.lock().await;

        let mut candles = candles_guard
            .iter_mut()
            .filter(|c| **c == record)
            .peekable();
        if candles.peek().is_some() {
            for candle in candles {
                (*candle).update(&record);
            }
        } else {
            for period in self._config.periods.clone().into_iter() {
                candles_guard.push(Candle::new(&record, period));
            }
        }

        Ok(vec![])
    }

    fn metrics(&self) -> ProcessorMetrics {
        self.metrics.clone()
    }

    fn config(&self) -> Box<dyn ProcessorConfig + Send + Sync> {
        Box::new(self._config.clone())
    }
}

pub fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}

#[derive(Clone, Debug, Deserialize)]
pub struct CandleProcessorConfig {
    #[serde(rename = "concurrency")]
    pub _concurrency: usize,
    #[serde(rename = "input")]
    pub _input: PostChannel,
    #[serde(rename = "output")]
    pub _output: PostChannel,
    pub name: String,
    pub periods: Vec<Duration>,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    pub filter: PostFilter,
}

#[async_trait]
impl ProcessorConfig for CandleProcessorConfig {
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
        let uuid = Uuid::new_v4();

        // Initialize flush timer
        let timer = Timer::new(
            order.clone(),
            Order::Flush { uuid, reason: None },
            self.periods.clone(),
            format!("{}_timer", self.name),
            meter.clone(),
        );

        let processor: Arc<Box<dyn Processor + Sync + Send>> = Arc::new(Box::new(
            CandleProcessor::try_new(self.clone(), meter, timer, uuid)?,
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
