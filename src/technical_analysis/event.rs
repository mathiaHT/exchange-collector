//! Dynamodb exporter

use chrono::{DateTime, Utc};
use rusoto_dynamodb::PutRequest;
use serde::{Deserialize, Serialize};
use tokio::time::Duration;
use uuid::Uuid;
use yata::core::IndicatorResult;

use crate::error::Error;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MarketProduct {
    pub source: String,
    pub channel: String,
    pub indicator: String,
    pub product: String,
    pub period: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketEvent {
    #[serde(rename = "Id")]
    pub id: Uuid,
    #[serde(rename = "Result")]
    pub result: IndicatorResult,
    #[serde(rename = "Product")]
    pub product: MarketProduct,
    #[serde(rename = "Datetime")]
    pub datetime: DateTime<Utc>,
}

impl MarketEvent {
    pub fn has_changed(&self, other: &Self) -> bool {
        let changed = self.product.eq(&other.product)
            & self
                .result
                .signals()
                .iter()
                .zip(other.result.signals().iter())
                .any(|(new, old)| std::mem::discriminant(new) != std::mem::discriminant(old));

        log::debug!(
            "Change from {:?} to {:?} has changed {:?}.",
            self.result.signals(),
            other.result.signals(),
            changed
        );
        changed
    }
}

impl TryInto<PutRequest> for MarketEvent {
    type Error = Error;

    fn try_into(self) -> Result<PutRequest, Self::Error> {
        Ok(PutRequest {
            item: serde_dynamo::to_item(self)?,
        })
    }
}

impl PartialEq for MarketEvent {
    fn eq(&self, other: &Self) -> bool {
        self.product.eq(&other.product)
    }
}

#[cfg(test)]
mod tests {
    use super::MarketEvent;
    use rstest::*;

    #[rstest]
    #[case::changed(
        r#"
        {
            "Datetime": "2022-12-14T11:04:28.207161Z",
            "Id": "66df9de9-56b2-420a-b097-ffef2f1228bd",
            "Product": {
                "source": "source",
                "channel": "channel",
                "product": "product",
                "period": {
                    "secs": 30,
                    "nanos": 0
                },
                "indicator": "aroon"
            },
            "Result": {
                "signals": [{"Buy": 1}, "None", "None", "None"],
                "values": [2, 0, 0, 0],
                "length": [1, 1]
            }
        }
        "#,
        r#"
        {
            "Datetime": "2022-12-14T11:04:28.207161Z",
            "Id": "cdeb3bb2-efeb-455a-b936-4aa9d73f57df",
            "Result": {
                "signals": [{"Sell": 1}, "None", "None", "None"],
                "values": [2, 0, 0, 0],
                "length": [1, 1]
            },
            "Product": {
                "source": "source",
                "channel": "channel",
                "product": "product",
                "period": {
                    "secs": 30,
                    "nanos": 0
                },
                "indicator": "aroon"
            }
        }
        "#
    )]
    fn test_has_changed(#[case] first: &'static str, #[case] second: &'static str) {
        let first = serde_json::from_str::<MarketEvent>(first).unwrap();
        let second = serde_json::from_str::<MarketEvent>(second).unwrap();

        assert!(first.has_changed(&second));
    }
}
