use async_trait::async_trait;
use enum_dispatch::enum_dispatch;
use futures::{
    self, future,
    stream::{StreamExt, TryStreamExt},
};
use opentelemetry::{
    metrics::{Counter, Meter, ObservableGauge, Unit, UpDownCounter},
    KeyValue,
};
use std::{
    marker::{Send, Sync},
    sync::Arc,
};
use tokio::{sync::broadcast, task::JoinHandle, time::Instant};
use tokio_stream::wrappers::BroadcastStream;

use super::actor::{Actor, ActorMeter};
use super::channel::{Order, Post, PostChannel};
use super::error::Error;
use super::metrics::{instrument, metadata, unit};

pub trait ExporterMeter: ActorMeter {
    fn increment_receive_count(&self);

    fn increment_task_count(&self);

    fn decrement_task_count(&self);

    fn increment_flush_failed(&self);

    fn increment_flush_success(&self);

    fn increment_handle_success(&self);

    fn increment_handle_failed(&self);

    fn record_flush_duration(&self, value: f64);
}

#[derive(Clone)]
pub struct ExporterMetrics {
    metadata: [KeyValue; 1],
    /// Count the number of received messages
    receive_count: Counter<u64>,
    /// Count the current number of concurrent tasks
    task_count: UpDownCounter<i64>,
    /// Buffer flushing duration
    flush_duration: ObservableGauge<f64>,
    /// Count flush failure
    flush_failed: Counter<u64>,
    /// Count flush success
    flush_success: Counter<u64>,
    /// Count the number of data wrote in the buffer failed
    handle_failed: Counter<u64>,
    /// Count the number of data wrote in the buffer successfully
    handle_success: Counter<u64>,
}

impl ExporterMetrics {
    pub fn new(name: String, meter: Arc<Meter>) -> Self {
        Self {
            metadata: [KeyValue::new(metadata::ACTOR_NAME, name)],
            receive_count: meter
                .u64_counter(instrument::RECEIVE_COUNT)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            task_count: meter
                .i64_up_down_counter(instrument::TASK_COUNT)
                .with_unit(Unit::new(unit::TASK))
                .init(),
            flush_duration: meter
                .f64_observable_gauge(instrument::FLUSH_DURATION)
                .with_unit(Unit::new(unit::SECONDS))
                .init(),
            flush_failed: meter
                .u64_counter(instrument::FLUSH_FAILED)
                .with_unit(Unit::new(unit::TASK))
                .init(),
            flush_success: meter
                .u64_counter(instrument::FLUSH_SUCCESS)
                .with_unit(Unit::new(unit::TASK))
                .init(),
            handle_failed: meter
                .u64_counter(instrument::HANDLE_FAILED)
                .with_unit(Unit::new(unit::TASK))
                .init(),
            handle_success: meter
                .u64_counter(instrument::HANDLE_SUCCESS)
                .with_unit(Unit::new(unit::TASK))
                .init(),
        }
    }
}

impl ActorMeter for ExporterMetrics {}
impl ExporterMeter for ExporterMetrics {
    fn increment_receive_count(&self) {
        self.receive_count.add(1, &self.metadata);
    }

    fn increment_task_count(&self) {
        self.task_count.add(1, &self.metadata);
    }

    fn decrement_task_count(&self) {
        self.task_count.add(-1, &self.metadata);
    }

    fn increment_flush_failed(&self) {
        self.flush_failed.add(1, &self.metadata)
    }

    fn increment_flush_success(&self) {
        self.flush_success.add(1, &self.metadata)
    }

    fn increment_handle_success(&self) {
        self.handle_success.add(1, &self.metadata)
    }

    fn increment_handle_failed(&self) {
        self.handle_failed.add(1, &self.metadata)
    }

    fn record_flush_duration(&self, value: f64) {
        self.flush_duration.observe(value, &self.metadata)
    }
}

/// A trait for format specific datalake exporter
#[async_trait]
pub trait Exporter: Actor {
    async fn flush(&self) -> Result<(), Error>;
    async fn write(&self, chunk: Vec<Post>) -> Result<(), Error>;
    fn metrics(&self) -> ExporterMetrics;
    fn config(&self) -> Box<dyn ExporterConfig + Send + Sync>;
}

#[async_trait]
#[enum_dispatch]
pub trait ExporterConfig {
    fn concurrency(&self) -> usize;
    fn chunk_size(&self) -> usize;
    fn input(&self) -> PostChannel;
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
    >;
}

async fn exit_export(
    exporter: Arc<Box<dyn Exporter + Send + Sync>>,
    mut input: broadcast::Receiver<Post>,
) -> Result<(), Error> {
    let exporter = exporter.clone();

    while let Ok(post) = input.try_recv() {
        exporter.write(vec![post]).await.unwrap_or_else(|error| {
            exporter.metrics().increment_handle_failed();
            log::error!("Exporter failed to write post: {:?}", error)
        });
        exporter.metrics().increment_handle_success();
    }

    exporter.flush().await?;

    Ok(())
}

/// Run a datalake exporter
pub async fn launch_exporter(
    exporter: Arc<Box<dyn Exporter + Send + Sync>>,
    filter: Box<dyn Fn(&Post) -> bool + Send + Sync>,
    order: broadcast::Sender<Post>,
    input: broadcast::Sender<Post>,
) -> Result<(), Error> {
    log::debug!("Exporter start.");

    let exporter = exporter.clone();
    let mut order_receiver = order.subscribe();

    let stream =
        BroadcastStream::new(input.subscribe()).try_filter(|post| future::ready(filter(post)));

    let mut export = stream
        .ready_chunks(exporter.config().chunk_size())
        .for_each_concurrent(exporter.config().concurrency(), |msg| async {
            let exporter = exporter.clone();

            exporter.metrics().increment_task_count();
            exporter.metrics().increment_receive_count();

            let chunk = msg
                .into_iter()
                .filter_map(|p| p.ok())
                .collect::<Vec<Post>>();

            if let Err(error) = exporter.write(chunk.clone()).await {
                log::error!("Exporter failed to write post: {:?}", error);
                exporter.metrics().increment_handle_failed();
            } else {
                exporter.metrics().increment_handle_success();
            };

            exporter.metrics().decrement_task_count();
        });

    log::debug!("Exporter start writting..");

    loop {
        tokio::select! {
            result = &mut export => {
                log::info!("Exporter stream ended with: {:?}", result);
                break
            },
            msg = order_receiver.recv() => {
                log::debug!("Actor received order {:?}", msg);
                if let Ok(post) = msg {
                    match Order::try_from(post) {
                        Ok(Order::Flush{uuid, ..}) if exporter.is_me(uuid) => {
                            log::info!("Exporter {:?} received flush order.", uuid);
                            let start = Instant::now();
                            exporter.flush().await.map_err(|error| {
                                log::error!("{:?}", error);
                                exporter.metrics().increment_flush_failed();
                                error
                            })?;
                            exporter.metrics().increment_flush_success();
                            exporter.metrics().record_flush_duration(start.elapsed().as_secs_f64());
                            continue
                        },
                        Ok(Order::Exit{uuid, ..}) if exporter.is_me(uuid) => break,
                        Ok(_) => continue,
                        Err(error) => {
                            log::error!("{:?}", error);
                            break
                        },
                    }
                } else {
                    log::error!("{:?}", msg);
                    break
                }
            }
        }
    }

    exit_export(exporter.clone(), input.subscribe()).await?;
    log::info!("Exporter exited.");
    Ok(())
}
