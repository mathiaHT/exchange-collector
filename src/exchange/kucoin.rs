use super::common::{ExchangeConfig, OrderSide};
use crate::{
    error::Error,
    matches::MatchRecord,
    record::Record,
    utils::serde::{f64_from_string, i64_from_string},
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub const KUCOIN: &str = "kucoin";

pub fn get_time() -> u128 {
    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");

    since_the_epoch.as_millis()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceData {
    pub data: InstanceServers,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceServers {
    pub instance_servers: Vec<InstanceServer>,
    pub token: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceServer {
    pub ping_interval: i32,
    pub endpoint: String,
    pub protocol: String,
    pub encrypt: bool,
    pub ping_timeout: i32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "method", rename = "SUBSCRIBE")]
pub struct KucoinSubscription {
    pub id: i64,
    pub r#type: String,
    pub topic: String,
    pub private_channel: bool,
    pub response: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct KucoinPing {
    id: i64,
    r#type: String,
}

impl Default for KucoinPing {
    fn default() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            id: rng.gen::<i64>(),
            r#type: String::from("ping"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct KucoinConfig {
    #[serde(alias = "subscription")]
    _subscription: KucoinSubscription,
    #[serde(alias = "uri")]
    _uri: Url,
}

#[async_trait]
impl ExchangeConfig for KucoinConfig {
    fn builde_parsing(&self) -> Box<dyn Fn(&str) -> Result<Record, Error> + Send + Sync> {
        let parser = |msg: &str| -> Result<Record, Error> {
            if let Ok(record) = serde_json::from_str::<KucoinRecord>(msg) {
                Ok(record.build_record())
            } else {
                Err(Error::UnprocessableEvent(format!("Cannot parse {:?}", msg)))
            }
        };
        Box::new(parser)
    }

    async fn uri(&self) -> Result<String, Error> {
        let timestamp = get_time();
        let client = reqwest::Client::new();
        let resp: InstanceData = client.post(self._uri.clone()).send().await?.json().await?;
        resp.data
            .instance_servers
            .first()
            .map(|server| {
                let endpoint = server.endpoint.to_owned();
                format!(
                    "{}?token={}&[connectId={}]?acceptUserMessage=\"true\"",
                    endpoint, resp.data.token, timestamp
                )
            })
            .ok_or_else(|| {
                Error::InternalServerError("Cannot find kucoin ws endpoint.".to_string())
            })
    }
    fn ping(&self) -> Option<String> {
        serde_json::to_string(&KucoinPing::default()).ok()
    }

    fn subscription(&self) -> Result<String, Error> {
        serde_json::to_string(&self._subscription).map_err(Error::from)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KucoinMatch {
    sequence: String,
    symbol: String,
    side: OrderSide,
    #[serde(deserialize_with = "f64_from_string")]
    size: f64,
    #[serde(deserialize_with = "f64_from_string")]
    price: f64,
    taker_order_id: String,
    #[serde(deserialize_with = "i64_from_string")]
    time: i64,
    maker_order_id: String,
    trade_id: String,
}

impl From<KucoinMatch> for Record {
    fn from(value: KucoinMatch) -> Self {
        let time = Utc
            .timestamp_millis_opt(value.time)
            .single()
            .unwrap_or_else(Utc::now);
        Record::Match(MatchRecord {
            price: value.price,
            volume: value.size,
            product: value.symbol.to_string(),
            source: KUCOIN.to_string(),
            time,
            buyer_id: Some(value.maker_order_id),
            seller_id: Some(value.taker_order_id),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "type")]
pub enum KucoinMessageData {
    #[serde(rename = "matches", alias = "match")]
    Match(KucoinMatch),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KucoinRecord {
    pub data: KucoinMessageData,
}

impl KucoinRecord {
    pub fn build_record(self) -> Record {
        match self.data {
            KucoinMessageData::Match(msg) => msg.into(),
        }
    }
}
