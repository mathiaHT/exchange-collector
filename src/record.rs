use crate::{channel::Post, error::Error, matches::MatchRecord};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::collections::HashMap;

const CHANNEL: &str = "channel";
const PRODUCT: &str = "product";
const SOURCE: &str = "source";

/// Represent the behaviour of a market record
#[enum_dispatch]
pub trait PriceVolume {
    fn price(&self) -> f64;
    fn volume(&self) -> f64;
    fn product(&self) -> String;
    fn source(&self) -> String;
    fn channel(&self) -> String;
}

#[enum_dispatch(PriceVolume)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Record {
    #[serde(alias = "match", alias = "matches")]
    Match(MatchRecord),
}

impl TryInto<Post> for Record {
    type Error = Error;

    fn try_into(self) -> Result<Post, Self::Error> {
        let tag = format!(
            "record_{}_{}_{}",
            self.source(),
            self.channel(),
            self.product(),
        );

        let body_attributes: HashMap<String, String> = HashMap::from([
            (SOURCE.to_string(), self.source()),
            (PRODUCT.to_string(), self.product()),
            (CHANNEL.to_string(), self.channel()),
        ]);
        log::debug!("Emit post with tag: {:?}", tag);

        Ok(Post::new(serde_json::to_value(self)?, body_attributes, tag))
    }
}

impl TryFrom<Post> for Record {
    type Error = Error;

    fn try_from(post: Post) -> Result<Self, Self::Error> {
        let mut data = if let Value::Object(body) = post.body.clone() {
            body
        } else {
            return Err(Error::PostNotValidMap);
        };

        let source = post
            .body_attributes
            .get(&SOURCE.to_string())
            .ok_or_else(|| Error::SourceNotFound(format!("{:?}", post.body_attributes.clone())))?;

        data.insert(SOURCE.to_string(), Value::String(source.clone()));

        let event = serde_json::from_value::<Record>(data.into())?;
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::{MatchRecord, Record};
    use rstest::*;

    // from https://docs.cloud.coinbase.com/exchange/docs/channels#match
    #[rstest]
    #[case(
        r#"
    {
        "source": "coinbase",
        "type": "match",
        "product": "btcusd",
        "volume": "5.23512",
        "price": "400.23",
        "extra": "information",
        "time": "2023-07-09 09:00:31.633028105 UTC"
    }
    "#,
        400.23
    )]
    fn test_pattern_matching(#[case] data: &'static str, #[case] expected: f64) {
        log::info!("{:?}", serde_json::from_str::<Record>(data));
        match serde_json::from_str::<Record>(data) {
            Ok(Record::Match(MatchRecord { price: found, .. })) => {
                assert_eq!(found, expected)
            }
            _ => unimplemented!(),
        }
    }
}
