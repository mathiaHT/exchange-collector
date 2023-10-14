use async_trait::async_trait;
use enum_dispatch::enum_dispatch;
use futures::future;
use futures_util::{SinkExt, TryStreamExt};
use opentelemetry::metrics::Meter;
use serde::Deserialize;
use std::{pin::Pin, sync::Arc, vec};
use tokio::{
    sync::broadcast,
    task::{self, JoinHandle},
    time::Duration,
};
use tokio_tungstenite::tungstenite::Message as TMessage;
use uuid::Uuid;

use super::{
    binance::BinanceConfig,
    coinbase::CoinbaseConfig,
    common::{ExchangeConfig, ParsingFunction},
    kucoin::KucoinConfig,
};
use crate::{
    actor::Actor,
    channel::{launch_timer, Order, Post, PostChannel, Timer},
    error::Error,
    websocket::{
        get_handshake, launch_client, WebsocketClient, WebsocketReceiverConfig,
        WebsocketReceiverMetrics, WebsocketSource,
    },
};

/// Represents an subscription to send to a websocket server
#[enum_dispatch(ExchangeConfig)]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "source")]
pub enum ExchangeSpecificConfig {
    #[serde(rename = "binance")]
    Binance(BinanceConfig),
    #[serde(rename = "coinbase")]
    Coinbase(CoinbaseConfig),
    #[serde(rename = "kucoin")]
    Kucoin(KucoinConfig),
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExchangeReceiverConfig {
    pub exchange: ExchangeSpecificConfig,
    pub name: String,
    #[serde(rename = "concurrency")]
    pub _concurrency: usize,
    #[serde(rename = "output")]
    pub _output: PostChannel,
    #[serde(rename = "ping_period")]
    pub _ping_period: Option<Duration>,
}

#[async_trait]
impl WebsocketReceiverConfig for ExchangeReceiverConfig {
    fn concurrency(&self) -> usize {
        self._concurrency
    }

    fn output(&self) -> PostChannel {
        self._output
    }

    async fn init(&self) -> Result<Pin<Box<dyn WebsocketSource<TMessage>>>, Error> {
        log::info!("Subscription {}", self.name);
        let mut stream = get_handshake::<Error>(self.exchange.uri().await?.as_str())
            .await?
            .try_filter(|msg| future::ready(msg.is_text()))
            .sink_map_err(Error::Send)
            .map_err(Error::Read);

        log::debug!("Stream {} set up.", self.name);

        stream
            .send(TMessage::Text(self.exchange.subscription()?))
            .await?;

        log::info!("Websocket {} subscribed.", self.name);

        Ok(Box::pin(stream))
    }

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
    > {
        let uuid = Uuid::new_v4();
        log::info!("Creating receiver: {:?} {:?} ...", self.name, uuid);
        let timer = self._ping_period.map(|period| {
            log::info!("Build ping timer {}", self.name);
            Timer::new(
                order.clone(),
                Order::Ping { uuid, reason: None },
                vec![period],
                format!("{}_timer", self.name),
                meter.clone(),
            )
        });

        let receiver: Arc<Box<dyn WebsocketClient + Send + Sync>> =
            Arc::new(Box::new(ExchangeReceiver::try_new(
                self.clone(),
                meter,
                timer,
                uuid,
                self.exchange.builde_parsing(),
            )?));

        log::info!("Launching receiver: {:?} {:?}", self.name, uuid);

        let task = task::spawn(launch_client(
            receiver.clone(),
            self.concurrency(),
            order,
            output,
        ));

        log::info!("Receiver created: {:?} ...", self.name);

        Ok((receiver, task))
    }

    fn ping(&self) -> Option<String> {
        self.exchange.ping()
    }
}

pub struct ExchangeReceiver {
    _config: ExchangeReceiverConfig,
    _uuid: Uuid,
    _metrics: WebsocketReceiverMetrics,
    _parser: ParsingFunction,
}

impl ExchangeReceiver {
    fn try_new(
        config: ExchangeReceiverConfig,
        meter: Arc<Meter>,
        timer: Option<Timer>,
        uuid: Uuid,
        parser: ParsingFunction,
    ) -> Result<Self, Error> {
        if let Some(inner) = timer {
            task::spawn(launch_timer(inner));
        }

        Ok(Self {
            _config: config.clone(),
            _uuid: uuid,
            _metrics: WebsocketReceiverMetrics::new(config.name, meter),
            _parser: parser,
        })
    }
}

impl Actor for ExchangeReceiver {
    fn name(&self) -> String {
        self._config.name.to_string()
    }

    fn uuid(&self) -> Uuid {
        self._uuid
    }
}

#[async_trait]
impl WebsocketClient for ExchangeReceiver {
    fn handle(&self, tmessage: &TMessage) -> Result<Vec<Post>, Error> {
        if let TMessage::Text(message) = tmessage {
            let post = (self._parser)(message)?.try_into()?;
            Ok(vec![post])
        } else {
            log::error!("Not a tungstenite::Message::Text: {:?}", tmessage);
            Err(Error::ExpectedTungsteniteMessage)
        }
    }

    fn metrics(&self) -> WebsocketReceiverMetrics {
        self._metrics.clone()
    }

    fn config(&self) -> Box<dyn WebsocketReceiverConfig + Send + Sync> {
        Box::new(self._config.clone())
    }
}
