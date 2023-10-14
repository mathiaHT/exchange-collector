use crate::{
    error::Error,
    metrics::{instrument, metadata, unit},
    utils::serde::regex_opt_from_string,
};
use futures::{stream::FuturesUnordered, StreamExt};
use opentelemetry::{
    metrics::{Counter, Meter, Unit},
    KeyValue,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Debug, sync::Arc};
use tokio::{
    sync::broadcast,
    time::{self, Duration},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub struct Post {
    pub body: serde_json::Value,
    pub body_attributes: HashMap<String, String>,
    pub tag: String,
}

impl Post {
    pub fn new(
        body: serde_json::Value,
        body_attributes: HashMap<String, String>,
        tag: String,
    ) -> Self {
        Self {
            body,
            body_attributes,
            tag,
        }
    }
}

#[derive(Clone, Default, Debug, Deserialize)]
pub struct PostFilter {
    #[serde(default)]
    pub body_attributes: HashMap<String, Vec<String>>,
    #[serde(default, deserialize_with = "regex_opt_from_string")]
    pub tag_pattern: Option<Regex>,
}

impl PostFilter {
    pub fn is_match(&self, post: &Post) -> bool {
        let mut superset = true;
        for (k, possible) in self.body_attributes.iter() {
            superset = post
                .body_attributes
                .get(k)
                .map(|v| possible.contains(v))
                .unwrap_or(false);

            if !superset {
                break;
            }
        }
        self.tag_pattern
            .as_ref()
            .map_or(true, |p| p.is_match(&post.tag))
            && superset
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChannelMessage<T> {
    pub body: Vec<T>,
    pub tag: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum OrderReason {
    Period(Duration),
    Text(String),
}

/// Connectors can receive these orders
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Order {
    Exit {
        uuid: Uuid,
        reason: Option<OrderReason>,
    },
    Flush {
        uuid: Uuid,
        reason: Option<OrderReason>,
    },
    Reset {
        uuid: Uuid,
        reason: Option<OrderReason>,
    },
    Ping {
        uuid: Uuid,
        reason: Option<OrderReason>,
    },
}

impl Order {
    fn with_reason(self, reason: OrderReason) -> Self {
        match self {
            Self::Exit { uuid, .. } => Self::Exit {
                uuid,
                reason: Some(reason),
            },
            Self::Flush { uuid, .. } => Self::Flush {
                uuid,
                reason: Some(reason),
            },
            Self::Reset { uuid, .. } => Self::Reset {
                uuid,
                reason: Some(reason),
            },
            Self::Ping { uuid, .. } => Self::Ping {
                uuid,
                reason: Some(reason),
            },
        }
    }
}

impl TryInto<Post> for Order {
    type Error = Error;

    fn try_into(self) -> Result<Post, Self::Error> {
        let post = Post::new(
            serde_json::to_value(self)?,
            HashMap::new(),
            String::from("order"),
        );

        Ok(post)
    }
}

impl TryFrom<Post> for Order {
    type Error = Error;

    fn try_from(value: Post) -> Result<Self, Self::Error> {
        let order = serde_json::from_value::<Self>(value.body)?;
        Ok(order)
    }
}

#[derive(Clone)]
pub struct TimerMetrics {
    metadata: [KeyValue; 1],
    /// Count success message handling
    send_success: Counter<u64>,
    /// Count failure message handling
    send_failed: Counter<u64>,
}

impl TimerMetrics {
    pub fn new(name: String, meter: Arc<Meter>) -> Self {
        Self {
            metadata: [KeyValue::new(metadata::ACTOR_NAME, name)],
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

    fn increment_send_success(&self) {
        self.send_success.add(1, &self.metadata);
    }

    fn increment_send_failed(&self) {
        self.send_failed.add(1, &self.metadata);
    }
}

pub struct Timer {
    order_sender: broadcast::Sender<Post>,
    order: Order,
    periods: Vec<Duration>,
    metrics: TimerMetrics,
}

impl Timer {
    pub fn new(
        order_sender: broadcast::Sender<Post>,
        order: Order,
        periods: Vec<Duration>,
        name: String,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            order_sender,
            order,
            periods,
            metrics: TimerMetrics::new(name, meter),
        }
    }

    fn send(&self, period: Duration) -> Result<(), Error> {
        log::debug!("Send order {:?}", self.order);
        self.order_sender.send(
            self.order
                .clone()
                .with_reason(OrderReason::Period(period))
                .try_into()?,
        )?;
        Ok(())
    }
}

pub async fn launch_timer(timer: Timer) -> Result<(), Error> {
    let mut intervals = FuturesUnordered::new();
    let timer = Arc::new(timer);

    for period in timer.periods.clone().into_iter() {
        let cloned = Arc::clone(&timer);
        intervals.push(tokio::spawn(async move {
            let mut interval = time::interval(period);

            // dont send order when launching timer
            interval.tick().await;

            loop {
                interval.tick().await;
                match cloned.send(interval.period()) {
                    Ok(_) => cloned.metrics.increment_send_success(),
                    Err(_) => cloned.metrics.increment_send_failed(),
                }
            }
        }))
    }

    loop {
        tokio::select! {
            Some(Ok(_)) = intervals.next() => {},
            else => break,
        }
    }

    Ok(())
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub enum PostChannel {
    #[serde(rename = "data")]
    Data,
    #[serde(rename = "signal")]
    Signal,
    #[serde(rename = "order")]
    Order,
}

#[cfg(test)]
mod tests {
    use super::Post;
    use regex::Regex;
    use rstest::*;
    use std::collections::HashMap;

    #[rstest]
    #[case::contains(Box::new(|post: &Post| -> bool {
        let sources: Vec<String> = vec!["coinbase".to_string()];
        let channels: Vec<String> = vec!["matches".to_string(), "match".to_string()];

        let check_source = if let Some(source) = post.body_attributes.get::<String>(&"source".to_string()) {
            sources.contains(source)
        } else {
            false
        };

        let check_channel = if let Some(channel) =
            post.body_attributes.get::<String>(&"channel".to_string())
        {
            channels.contains(channel)
        } else {
            false
        };
        check_source && check_channel
    })
    )]
    #[case::tag(Box::new(|post: &Post| -> bool {
        Regex::new(r"^coinbase_.*").unwrap().is_match(post.tag.as_str())
    })
    )]
    fn test_filter_post(#[case] filter: Box<dyn Fn(&Post) -> bool + Send + Sync>) {
        let body = serde_json::json!({
            "type": "match",
            "trade_id": 10,
            "sequence": 50,
            "maker_order_id": "ac928c66-ca53-498f-9c13-a110027a60e8",
            "taker_order_id": "132fb6ae-456b-4654-b4e0-d681ac05cea1",
            "time": "2014-11-07T08:19:27.028459Z",
            "product_id": "BTC-USD",
            "size": "5.23512",
            "price": "400.23",
            "side": "sell"
        });

        let mut body_attributes: HashMap<String, String> = HashMap::new();
        body_attributes.insert("source".to_string(), "coinbase".to_string());
        body_attributes.insert("channel".to_string(), "match".to_string());

        let tag = String::from("coinbase_match");

        let post = Post::new(body, body_attributes, tag);

        assert!(filter(&post));
    }
}
