use async_trait::async_trait;
use enum_dispatch::enum_dispatch;
use futures::{future, Sink, SinkExt, Stream};
use futures_util::stream::StreamExt;
use opentelemetry::{
    metrics::{Counter, Meter, Unit, UpDownCounter},
    KeyValue,
};
use std::{
    marker::{Send, Sync},
    pin::Pin,
    sync::Arc,
};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message as TMessage;

use http::StatusCode;
use native_tls::TlsConnector;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};
use url::Url;

use super::actor::{Actor, ActorMeter};
use super::channel::{Order, Post, PostChannel};
use super::error::Error;
use super::metrics::{instrument, metadata, unit};

pub type Handshake = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Get websocket handshake
pub async fn get_handshake<Er: From<Error>>(uri: &str) -> Result<Handshake, Er> {
    log::info!("Initiate handshake to {}.", uri);
    let url = Url::parse(uri).map_err(Error::from)?;
    let connector = TlsConnector::new().unwrap();
    let (stream, response) =
        connect_async_tls_with_config(url, None, false, Some(Connector::NativeTls(connector)))
            .await
            .map_err(Error::from)?;

    match response.status() {
        StatusCode::SWITCHING_PROTOCOLS => {
            log::debug!("Server has upgraded protocol.");
            Ok(stream)
        }
        StatusCode::OK => Ok(stream),
        _ => Err(Error::Http(format!("Status: {:?}", response.status())))?,
    }
}

#[async_trait]
pub trait WebsocketSink<Msg>: Sink<Msg, Error = Error> + Unpin + Send {}
impl<Msg, Str> WebsocketSink<Msg> for Str where Str: Sink<Msg, Error = Error> + Unpin + Send {}

#[async_trait]
pub trait WebsocketStream<Msg>: Stream<Item = Result<Msg, Error>> + Unpin + Send {}
impl<Msg, Str> WebsocketStream<Msg> for Str where
    Str: Stream<Item = Result<Msg, Error>> + Unpin + Send
{
}

#[async_trait]
pub trait WebsocketSource<Msg>: WebsocketSink<Msg> + WebsocketStream<Msg> {}
impl<Msg, Str> WebsocketSource<Msg> for Str where Str: WebsocketSink<Msg> + WebsocketStream<Msg> {}

#[async_trait]
#[enum_dispatch]
pub trait WebsocketReceiverConfig {
    fn concurrency(&self) -> usize;
    /// Subscribe to a websocket stream
    async fn init(&self) -> Result<Pin<Box<dyn WebsocketSource<TMessage>>>, Error>;
    fn output(&self) -> PostChannel;
    async fn build(
        &self,
        meter: Arc<Meter>,
        order: broadcast::Sender<Post>,
        output: broadcast::Sender<Post>,
    ) -> Result<
        (
            Arc<Box<dyn WebsocketClient + Send + Sync>>,
            JoinHandle<Result<(), Error>>,
        ),
        Error,
    >;
    fn ping(&self) -> Option<String>;
}

#[async_trait]
pub trait WebsocketClient: Actor {
    /// Handle raw websocket messages
    fn handle(&self, tmessage: &TMessage) -> Result<Vec<Post>, Error>;
    fn metrics(&self) -> WebsocketReceiverMetrics;
    fn config(&self) -> Box<dyn WebsocketReceiverConfig + Send + Sync>;
}

pub trait WebsocketReceiverMeter: ActorMeter {
    fn increment_receive_count(&self);

    fn increment_task_count(&self);

    fn decrement_task_count(&self);

    fn increment_handle_success(&self);

    fn increment_handle_failed(&self);

    fn increment_send_success(&self);

    fn increment_send_failed(&self);

    fn increment_start_stream(&self);
}

#[derive(Clone)]
pub struct WebsocketReceiverMetrics {
    metadata: [KeyValue; 1],
    /// Count the number of received messages
    receive_count: Counter<u64>,
    /// Count the current number of concurrent tasks
    task_count: UpDownCounter<i64>,
    /// Count success message handling
    handle_success: Counter<u64>,
    /// Count failure message handling
    handle_failed: Counter<u64>,
    /// Count success message handling
    send_success: Counter<u64>,
    /// Count failure message handling
    send_failed: Counter<u64>,
    /// Count start of stream
    start_stream: Counter<u64>,
}

impl WebsocketReceiverMetrics {
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
            handle_success: meter
                .u64_counter(instrument::HANDLE_SUCCESS)
                .with_unit(Unit::new(unit::MESSAGE))
                .init(),
            handle_failed: meter
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
            start_stream: meter
                .u64_counter(instrument::START_STREAM)
                .with_unit(Unit::new(unit::TASK))
                .init(),
        }
    }
}

impl ActorMeter for WebsocketReceiverMetrics {}
impl WebsocketReceiverMeter for WebsocketReceiverMetrics {
    fn increment_receive_count(&self) {
        self.receive_count.add(1, &self.metadata);
    }

    fn increment_task_count(&self) {
        self.task_count.add(1, &self.metadata);
    }

    fn decrement_task_count(&self) {
        self.task_count.add(-1, &self.metadata);
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

    fn increment_start_stream(&self) {
        self.start_stream.add(1, &self.metadata);
    }
}

fn send_output(
    receiver: Arc<Box<dyn WebsocketClient + Send + Sync>>,
    posts: Vec<Post>,
    output: broadcast::Sender<Post>,
) {
    for signal in posts {
        match output.send(signal) {
            Ok(_) => receiver.metrics().increment_send_success(),
            Err(error) => {
                receiver.metrics().increment_send_failed();
                log::error!("Send failed: {:?}", error);
            }
        }
    }
}

/// Websocket receiver task
pub async fn launch_client(
    client: Arc<Box<dyn WebsocketClient + Send + Sync>>,
    concurrency: usize,
    order_sender: broadcast::Sender<Post>,
    output: broadcast::Sender<Post>,
) -> Result<(), Error> {
    log::debug!("Receiver start for {}.", client.name());

    // Clone shared structure when entering spawned function
    let mut order_receiver = order_sender.subscribe();

    // Subscribe to the websocket
    let (mut sink, mut stream) = client.config().init().await?.split();

    let handle = Box::new(|data: Result<TMessage, Error>| {
        let client = client.clone();
        let output = output.clone();

        client.metrics().increment_task_count();
        client.metrics().increment_receive_count();

        if let Ok(tmessage) = data {
            match client.handle(&tmessage) {
                Ok(posts) => {
                    client.metrics().increment_handle_success();
                    send_output(client.clone(), posts, output);
                }
                Err(Error::UnprocessableEvent { .. }) => (),
                Err(error) => {
                    client.metrics().increment_handle_failed();
                    log::error!(
                        "Failed {} to write {:?}: {:?}",
                        client.name(),
                        tmessage,
                        error
                    );
                }
            };
        } else {
            log::error!("Stream {} error: {:?}", client.name(), data);
            client.metrics().increment_handle_failed();
        }
        client.metrics().decrement_task_count();
        future::ready(())
    });

    let mut result = stream.for_each_concurrent(concurrency, &handle);

    log::info!("Receiver listening {}.", client.name());
    loop {
        tokio::select! {
            () = &mut result => {
                log::info!("Relaunch ended stream: {}.", client.name());
                (sink, stream) = client.config().init().await?.split();
                result = stream.for_each_concurrent(
                    concurrency, &handle,
                );
                client.metrics().increment_start_stream();
                continue
            },
            order_event = order_receiver.recv() => {
                log::debug!("Actor received order {:?}", order_event);
                if let Ok(post) = order_event {
                    match Order::try_from(post) {
                        Ok(Order::Exit{uuid, ..}) if client.is_me(uuid) => break,
                        Ok(Order::Ping{uuid, ..}) if client.is_me(uuid) => {
                            if let Some(ping) = client.config().ping() {
                                sink.send(TMessage::Text(ping)).await?;
                            }
                        }
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

    log::info!("Receiver {} exited.", client.name());
    Ok(())
}
