//! Channels defined by coinbase websocket server
//! Coinbase api <https://docs.cloud.coinbase.com/exchange/docs/overview>

use super::common::{ExchangeConfig, OrderSide, Symbol};
use crate::{
    error::Error,
    matches::MatchRecord,
    record::Record,
    utils::serde::{f64_from_string, f64_opt_from_string, uuid_from_string, uuid_opt_from_string},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, string::ToString};
use uuid::Uuid;

pub const COINBASE: &str = "coinbase";

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type", rename = "error")]
pub struct ErrorMessage {
    pub message: String,
    pub reason: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type", rename = "subscriptions")]
pub struct Subscriptions {
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum CoinbaseAction {
    #[serde(rename = "subscribe")]
    Subscription(CoinbaseSubscription),
    #[serde(rename = "unsubscribe")]
    Unsubscription(Unsubscription),
}

#[derive(Clone, Deserialize, Debug, Serialize)]
pub struct Auth {
    pub signature: String,
    pub key: String,
    pub passphrase: String,
    pub timestamp: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename = "subscribe")]
pub struct CoinbaseSubscription {
    pub product_ids: Vec<String>,
    pub channels: Vec<Channel>,
    #[serde(flatten)]
    pub auth: Option<Auth>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoinbaseConfig {
    #[serde(alias = "subscription")]
    pub _subscription: CoinbaseSubscription,
    #[serde(alias = "uri")]
    pub _uri: url::Url,
}

#[async_trait]
impl ExchangeConfig for CoinbaseConfig {
    fn builde_parsing(&self) -> Box<dyn Fn(&str) -> Result<Record, Error> + Send + Sync> {
        let parser = |msg: &str| -> Result<Record, Error> {
            serde_json::from_str::<CoinbaseRecord>(msg)?.build_record()
        };
        Box::new(parser)
    }

    async fn uri(&self) -> Result<String, Error> {
        Ok(self._uri.to_string())
    }

    fn subscription(&self) -> Result<String, Error> {
        serde_json::to_string(&self._subscription).map_err(Error::from)
    }

    fn ping(&self) -> Option<String> {
        None
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Unsubscription {
    pub product_ids: Vec<String>,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Deserialize, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Channel {
    Name(ChannelType),
    WithProduct {
        name: ChannelType,
        product_ids: Vec<String>,
    },
}

#[derive(Clone, Deserialize, Debug, PartialEq, Serialize)]
pub enum ChannelType {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "heartbeat")]
    Heartbeat,
    #[serde(rename = "level2update")]
    Level2Update,
    #[serde(rename = "matches", alias = "match")]
    Match,
    #[serde(rename = "level2snapshot")]
    Level2Snapshot,
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "ticker")]
    Ticker,
    #[serde(rename = "user")]
    User,
}

/// Represents a message from coinbase match channel.
/// <https://docs.cloud.coinbase.com/exchange/docs/channels#match>
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CoinbaseMatch {
    pub trade_id: usize,
    pub sequence: usize,
    #[serde(deserialize_with = "uuid_from_string")]
    pub maker_order_id: Uuid,
    #[serde(deserialize_with = "uuid_from_string")]
    pub taker_order_id: Uuid,
    pub time: DateTime<Utc>,
    pub product_id: Symbol,
    #[serde(deserialize_with = "f64_from_string")]
    pub size: f64,
    #[serde(deserialize_with = "f64_from_string")]
    pub price: f64,
    pub side: OrderSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taker_user_id: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "uuid_opt_from_string")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taker_profile_id: Option<Uuid>,
    #[serde(default)]
    #[serde(deserialize_with = "f64_opt_from_string")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taker_fee_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker_user_id: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "uuid_opt_from_string")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker_profile_id: Option<Uuid>,
    #[serde(default)]
    #[serde(deserialize_with = "f64_opt_from_string")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker_fee_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "uuid_opt_from_string")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<Uuid>,
}

impl From<CoinbaseMatch> for Record {
    fn from(val: CoinbaseMatch) -> Self {
        Record::Match(MatchRecord {
            price: val.price,
            volume: val.size,
            product: val.product_id.to_string(),
            source: COINBASE.to_string(),
            time: val.time,
            seller_id: val.taker_user_id,
            buyer_id: val.maker_user_id,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum CoinbaseRecord {
    #[serde(rename = "matches", alias = "match", alias = "last_match")]
    Match(CoinbaseMatch),
    #[serde(rename = "heartbeat")]
    Hearbeat {
        sequence: usize,
        last_trade_id: usize,
        product_id: String,
        time: DateTime<Utc>,
    },
}

impl CoinbaseRecord {
    pub fn build_record(self) -> Result<Record, Error> {
        match self {
            Self::Match(msg) => Ok(msg.into()),
            _ => Err(Error::UnprocessableEvent(String::default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    // from https://docs.cloud.coinbase.com/exchange/docs/channels#match
    #[rstest]
    #[case(
        r#"
    {
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
    }
    "#,
        "matches"
    )]
    fn test_pattern_matching(#[case] data: &'static str, #[case] expected: &'static str) {
        let record = serde_json::from_str::<CoinbaseRecord>(data).unwrap();

        assert_eq!(
            serde_json::to_value(record).unwrap().get("type").unwrap(),
            expected
        );
    }

    // from https://docs.cloud.coinbase.com/exchange/docs/channels#match
    #[rstest]
    #[case(
        r#"
    {
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
    }
    "#,
        9
    )]
    #[case(
        r#"
    {
        "type": "match",
        "trade_id": 10,
        "sequence": 50,
        "maker_order_id": "ac928c66-ca53-498f-9c13-a110027a60e8",
        "taker_order_id": "132fb6ae-456b-4654-b4e0-d681ac05cea1",
        "time": "2014-11-07T08:19:27.028459Z",
        "product_id": "BTC-USD",
        "size": "5.23512",
        "price": "400.23",
        "side": "sell",
        "taker_user_id": "5844eceecf7e803e259d0365",
        "user_id": "5844eceecf7e803e259d0365",
        "taker_profile_id": "765d1549-9660-4be2-97d4-fa2d65fa3352",
        "profile_id": "765d1549-9660-4be2-97d4-fa2d65fa3352",
        "taker_fee_rate": "0.005"
    }
    "#,
        14
    )]
    fn test_coinbase_match(#[case] data: &'static str, #[case] length: usize) {
        match serde_json::from_str::<CoinbaseRecord>(data).unwrap() {
            CoinbaseRecord::Match(msg @ CoinbaseMatch { .. }) => {
                assert_eq!(
                    serde_json::to_value(msg.clone())
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .len(),
                    length
                )
            }
            _ => unimplemented!(),
        }
    }

    #[fixture]
    fn subscribe_data() -> &'static str {
        r#"
        {
            "type": "subscribe",
            "product_ids": [
                "ETH-USD",
                "ETH-EUR"
            ],
            "channels": [
                "level2update",
                "heartbeat",
                {
                    "name": "ticker",
                    "product_ids": [
                        "ETH-BTC",
                        "ETH-USD"
                    ]
                }
            ]
        }
        "#
    }

    #[rstest]
    fn test_subscribe(subscribe_data: &str) {
        let subscribe = serde_json::from_str::<CoinbaseAction>(subscribe_data).unwrap();

        match subscribe.clone() {
            CoinbaseAction::Subscription(action @ CoinbaseSubscription { .. }) => {
                let value = serde_json::to_value(action).unwrap();
                assert_eq!(
                    value.clone().get("type").unwrap().as_str(),
                    Some("subscribe")
                );
                assert_eq!(value.clone().get("source"), None);
            }
            _ => unimplemented!(),
        }
        assert_eq!(
            serde_json::to_value(subscribe).unwrap(),
            serde_json::from_str::<serde_json::Value>(subscribe_data).unwrap()
        )
    }

    #[rstest]
    #[case::full(r#""full""#, ChannelType::Full)]
    #[case::heartbeat(r#""heartbeat""#, ChannelType::Heartbeat)]
    #[case::level2update(r#""level2update""#, ChannelType::Level2Update)]
    #[case::level2snapshot(r#""level2snapshot""#, ChannelType::Level2Snapshot)]
    #[case::matches(r#""matches""#, ChannelType::Match)]
    #[case::status(r#""status""#, ChannelType::Status)]
    #[case::ticker(r#""ticker""#, ChannelType::Ticker)]
    #[case::user(r#""user""#, ChannelType::User)]
    fn test_channel_type(#[case] input: &'static str, #[case] expected: ChannelType) {
        assert_eq!(
            serde_json::from_str::<ChannelType>(input).unwrap(),
            expected
        )
    }
}
