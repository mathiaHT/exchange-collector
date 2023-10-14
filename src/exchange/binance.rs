use super::common::{ExchangeConfig, Symbol};
use crate::{error::Error, matches::MatchRecord, record::Record, utils::serde::f64_from_string};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::string::ToString;

pub const BINANCE: &str = "binance";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BinanceSubscription {
    pub method: Option<String>,
    pub params: Vec<String>,
    pub id: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinanceConfig {
    #[serde(alias = "subscription")]
    pub _subscription: BinanceSubscription,
    #[serde(alias = "uri")]
    pub _uri: url::Url,
}

#[async_trait]
impl ExchangeConfig for BinanceConfig {
    fn builde_parsing(&self) -> Box<dyn Fn(&str) -> Result<Record, Error> + Send + Sync> {
        let parser = |msg: &str| -> Result<Record, Error> {
            serde_json::from_str::<BinanceRecord>(msg)?.build_record()
        };
        Box::new(parser)
    }

    async fn uri(&self) -> Result<String, Error> {
        Ok(self._uri.to_string())
    }

    fn subscription(&self) -> Result<String, Error> {
        let result = serde_json::to_string(&self._subscription).map_err(Error::from);
        log::info!("{:?}", result);
        result
    }

    fn ping(&self) -> Option<String> {
        None
    }
}

/// Represents a message from binance trade channel.
// <https://github.com/binance/binance-spot-api-docs/blob/master/web-socket-streams.md#trade-streams>
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Trade {
    #[serde(alias = "E")]
    event_time: i64,
    #[serde(alias = "s")]
    symbol: Symbol,
    #[serde(alias = "t")]
    trade_id: u64,
    #[serde(alias = "p", deserialize_with = "f64_from_string")]
    price: f64,
    #[serde(alias = "q", deserialize_with = "f64_from_string")]
    quantity: f64,
    #[serde(alias = "b")]
    buyer_order_id: u64,
    #[serde(alias = "a")]
    seller_order_id: u64,
    #[serde(alias = "T")]
    trade_order_time: i64,
    #[serde(alias = "m")]
    is_buyer_maker: bool,
    #[serde(skip, alias = "M")]
    m_ignore: bool,
}

impl From<Trade> for Record {
    fn from(val: Trade) -> Self {
        let time = Utc
            .timestamp_millis_opt(val.trade_order_time)
            .single()
            .unwrap_or_else(Utc::now);
        Record::Match(MatchRecord {
            price: val.price,
            volume: val.quantity,
            product: val.symbol.to_string(),
            source: BINANCE.to_string(),
            time,
            buyer_id: Some(val.buyer_order_id.to_string()),
            seller_id: Some(val.seller_order_id.to_string()),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename(serialize = "event_type"), tag = "e")]
pub enum BinanceRecord {
    #[serde(rename = "matches", alias = "trade")]
    Trade(Trade),
}

impl BinanceRecord {
    pub fn build_record(self) -> Result<Record, Error> {
        match self {
            Self::Trade(msg) => Ok(msg.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Record;
    use super::*;
    use rstest::*;

    #[rstest]
    #[case(
        r#"
        {
            "e": "trade",
            "E": 1672515782136,
            "s": "BTCEUR",
            "t": 12345,
            "p": "0.001",
            "q": "100",
            "b": 88,
            "a": 50,
            "T": 1672515782136,
            "m": true,
            "M": true
        }
    "#
    )]
    fn test_binance_trade(#[case] data: &'static str) {
        let message = serde_json::from_str::<BinanceRecord>(data).unwrap();

        matches!(message.build_record(), Ok(Record::Match(..)));
    }
}
