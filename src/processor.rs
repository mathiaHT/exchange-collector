use async_trait::async_trait;
use enum_dispatch::enum_dispatch;
use futures::{
    future,
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

pub trait ProcessorMeter: ActorMeter {
    fn increment_init_count(&self);

    fn increment_receive_count(&self);

    fn increment_not_supported_count(&self);

    fn increment_task_count(&self);

    fn decrement_task_count(&self);

    fn increment_flush_success(&self);

    fn increment_flush_failed(&self);

    fn increment_handle_success(&self);

    fn increment_handle_failed(&self);

    fn increment_send_success(&self);

    fn increment_send_failed(&self);

    fn record_flush_duration(&self, value: f64);
}

#[derive(Clone)]
pub struct ProcessorMetrics {
    metadata: [KeyValue; 1],
    /// Count the number of received messages
    init_count: Counter<u64>,
    /// Count the number of received messages
    receive_count: Counter<u64>,
    /// Count the number of not supported messages
    not_supported_count: Counter<u64>,
    /// Count the current number of concurrent tasks
    task_count: UpDownCounter<i64>,
    /// Count success message handling
    flush_success: Counter<u64>,
    /// Count failure message handling
    flush_failed: Counter<u64>,
    /// Buffer flushing duration
    flush_duration: ObservableGauge<f64>,
    /// Count success message handling
    handle_success: Counter<u64>,
    /// Count failure message handling
    handle_failed: Counter<u64>,
    /// Count success message handling
    send_success: Counter<u64>,
    /// Count failure message handling
    send_failed: Counter<u64>,
}

impl ProcessorMetrics {
    pub fn new(name: String, meter: Arc<Meter>) -> Self {
        Self {
            metadata: [KeyValue::new(metadata::ACTOR_NAME, name)],
            init_count: meter
                .u64_counter(instrument::INIT_COUNT)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            receive_count: meter
                .u64_counter(instrument::RECEIVE_COUNT)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            not_supported_count: meter
                .u64_counter(instrument::NOT_SUPPORTED)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            task_count: meter
                .i64_up_down_counter(instrument::TASK_COUNT)
                .with_unit(Unit::new(unit::TASK))
                .init(),
            handle_success: meter
                .u64_counter(instrument::HANDLE_SUCCESS)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            handle_failed: meter
                .u64_counter(instrument::HANDLE_FAILED)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            flush_duration: meter
                .f64_observable_gauge(instrument::FLUSH_DURATION)
                .with_unit(Unit::new(unit::SECONDS))
                .init(),
            flush_success: meter
                .u64_counter(instrument::HANDLE_SUCCESS)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            flush_failed: meter
                .u64_counter(instrument::HANDLE_FAILED)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            send_success: meter
                .u64_counter(instrument::SEND_FAILED)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            send_failed: meter
                .u64_counter(instrument::SEND_SUCCESS)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
        }
    }
}

impl ActorMeter for ProcessorMetrics {}
impl ProcessorMeter for ProcessorMetrics {
    fn increment_init_count(&self) {
        self.init_count.add(1, &self.metadata);
    }

    fn increment_receive_count(&self) {
        self.receive_count.add(1, &self.metadata);
    }

    fn increment_not_supported_count(&self) {
        self.not_supported_count.add(1, &self.metadata);
    }

    fn increment_task_count(&self) {
        self.task_count.add(1, &self.metadata);
    }

    fn decrement_task_count(&self) {
        self.task_count.add(-1, &self.metadata);
    }

    fn increment_flush_success(&self) {
        self.flush_success.add(1, &self.metadata);
    }

    fn increment_flush_failed(&self) {
        self.flush_failed.add(1, &self.metadata);
    }

    fn increment_handle_success(&self) {
        self.handle_success.add(1, &self.metadata);
    }

    fn increment_handle_failed(&self) {
        self.handle_failed.add(1, &self.metadata);
    }

    fn increment_send_success(&self) {
        self.send_success.add(1, &self.metadata);
    }

    fn increment_send_failed(&self) {
        self.send_failed.add(1, &self.metadata);
    }

    fn record_flush_duration(&self, value: f64) {
        self.flush_duration.observe(value, &self.metadata)
    }
}

#[async_trait]
pub trait Processor: Actor {
    async fn flush(&self, post: &Post) -> Result<Vec<Post>, Error>;
    async fn reset(&self) -> Result<Vec<Post>, Error>;
    async fn write(&self, post: &Post) -> Result<Vec<Post>, Error>;
    fn metrics(&self) -> ProcessorMetrics;
    fn config(&self) -> Box<dyn ProcessorConfig + Send + Sync>;
}

#[async_trait]
#[enum_dispatch]
pub trait ProcessorConfig {
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
    >;
    fn concurrency(&self) -> usize;
    fn input(&self) -> PostChannel;
    fn output(&self) -> PostChannel;
}

fn send_output(
    processor: Arc<Box<dyn Processor + Send + Sync>>,
    posts: Vec<Post>,
    output: broadcast::Sender<Post>,
) {
    for signal in posts {
        match output.send(signal) {
            Ok(_) => processor.metrics().increment_send_success(),
            Err(error) => {
                processor.metrics().increment_send_failed();
                log::error!("Send failed: {:?}", error);
            }
        }
    }
}

async fn exit_processor(
    processor: Arc<Box<dyn Processor + Send + Sync>>,
    mut input: broadcast::Receiver<Post>,
) -> Result<Vec<Post>, Error> {
    let mut posts: Vec<Post> = vec![];
    while let Ok(post) = input.try_recv() {
        posts.extend(processor.write(&post).await.unwrap_or_else(|error| {
            processor.metrics().increment_handle_failed();
            log::error!("Exporter failed to write post: {:?}", error);
            Vec::default()
        }));
        processor.metrics().increment_handle_success();
    }

    posts.extend(
        processor
            .flush(
                &Order::Exit {
                    uuid: processor.uuid(),
                    reason: None,
                }
                .try_into()?,
            )
            .await?,
    );

    Ok(posts)
}

pub async fn launch_processor(
    processor: Arc<Box<dyn Processor + Send + Sync>>,
    filter: Box<dyn Fn(&Post) -> bool + Send + Sync>,
    concurrency: usize,
    order: broadcast::Sender<Post>,
    input: broadcast::Sender<Post>,
    output: broadcast::Sender<Post>,
) -> Result<(), Error> {
    log::debug!("Mutable processor started.");

    // Clone shared structures when entering spawned function
    let processor = processor.clone();
    let handle_output = output.clone();
    let mut order_receiver = order.subscribe();

    let mut process = BroadcastStream::new(input.subscribe())
        .try_filter(|post: &Post| future::ready(filter(post)))
        .for_each_concurrent(concurrency, |received| async {
            // Clone shared structure when entering spawned function
            let processor = processor.clone();

            processor.metrics().increment_receive_count();
            processor.metrics().increment_task_count();

            if let Ok(post) = received {
                match processor.write(&post).await {
                    Ok(posts) => {
                        processor.metrics().increment_handle_success();
                        send_output(processor.clone(), posts, handle_output.clone());
                    }
                    Err(error) => {
                        processor.metrics().increment_handle_failed();
                        log::error!("Failed to write {:?}: {:?}", post, error);
                    }
                };
            } else {
                log::error!(
                    "{:?} received invalid data: {:?}",
                    processor.name(),
                    received
                );
                processor.metrics().increment_handle_failed();
            }
            processor.metrics().decrement_task_count();
        });

    loop {
        tokio::select! {
            _ = &mut process => {
                break Err(Error::UnexpectedEndOfStream)?
            },
            order_event = order_receiver.recv() => {
                log::debug!("Actor received order {:?}", order_event);
                if let Ok(post) = order_event {
                    match Order::try_from(post.clone()) {
                        Ok(Order::Reset{uuid, ..}) if processor.is_me(uuid) => {
                            let posts = processor.reset().await?;
                            send_output(processor.clone(), posts, output.clone());
                            continue
                        },
                        Ok(Order::Exit{uuid, ..}) if processor.is_me(uuid) => break,
                        Ok(Order::Flush{uuid, ..}) if processor.is_me(uuid) => {
                            let start = Instant::now();
                            let flush_result = processor.flush(&post).await;
                            processor.metrics().record_flush_duration(start.elapsed().as_secs_f64());
                            match flush_result {
                                Ok(posts) => {
                                    processor.metrics().increment_flush_success();
                                    send_output(processor.clone(), posts, output.clone())
                                }
                                Err(error) => {
                                    processor.metrics().increment_flush_failed();
                                    log::error!("Flush failed: {:?}", error);
                                }
                            }
                            continue
                        },
                        Ok(_) => continue,
                        Err(error) => {
                            log::error!("{:?}", error);
                            break
                        },
                    }
                } else {
                    log::error!("{:?}", order_event);
                    break
                }
            }
        }
    }

    let posts = exit_processor(processor.clone(), input.subscribe()).await?;
    send_output(processor.clone(), posts, output.clone());
    log::info!("Exporter exited.");
    Ok(())
}
