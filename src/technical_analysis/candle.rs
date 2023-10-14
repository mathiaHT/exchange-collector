use opentelemetry::{
    metrics::{Meter, ObservableGauge},
    KeyValue,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::time::Duration;
use yata::core::{ValueType, OHLCV};

use crate::{
    channel::Post,
    error::Error,
    metrics::{instrument, metadata},
    record::PriceVolume,
    utils::serde::f64_nan_from_string,
};

pub struct CandleMetrics {
    open: ObservableGauge<f64>,
    high: ObservableGauge<f64>,
    low: ObservableGauge<f64>,
    close: ObservableGauge<f64>,
    volume: ObservableGauge<f64>,
}

impl CandleMetrics {
    pub fn try_new(meter: Arc<Meter>) -> Result<Self, Error> {
        Ok(Self {
            open: meter
                .f64_observable_gauge(instrument::CANDLE_OPEN)
                .try_init()?,
            high: meter
                .f64_observable_gauge(instrument::CANDLE_HIGH)
                .try_init()?,
            low: meter
                .f64_observable_gauge(instrument::CANDLE_LOW)
                .try_init()?,
            close: meter
                .f64_observable_gauge(instrument::CANDLE_CLOSE)
                .try_init()?,
            volume: meter
                .f64_observable_gauge(instrument::CANDLE_VOLUME)
                .try_init()?,
        })
    }

    pub fn record(&self, candle: &Candle) {
        let attributes: &[KeyValue; 4] = &[
            KeyValue::new(metadata::PRODUCT, candle.product.to_lowercase()),
            KeyValue::new(metadata::SOURCE, candle.source.to_lowercase()),
            KeyValue::new(metadata::CHANNEL, candle.channel.to_lowercase()),
            KeyValue::new(metadata::CANDLE_PERIOD, candle.period.as_secs().to_string()),
        ];

        self.open.observe(candle.open, attributes);
        self.high.observe(candle.high, attributes);
        self.low.observe(candle.low, attributes);
        self.close.observe(candle.close, attributes);
        self.volume.observe(candle.volume, attributes);
    }
}

#[derive(Clone, Debug, Deserialize, PartialOrd, Serialize)]
pub struct Candle {
    pub source: String,
    pub channel: String,
    pub product: String,
    pub period: Duration,
    #[serde(deserialize_with = "f64_nan_from_string")]
    open: f64,
    #[serde(deserialize_with = "f64_nan_from_string")]
    high: f64,
    #[serde(deserialize_with = "f64_nan_from_string")]
    low: f64,
    #[serde(deserialize_with = "f64_nan_from_string")]
    close: f64,
    #[serde(deserialize_with = "f64_nan_from_string")]
    volume: f64,
}

impl Candle {
    pub fn new<R: PriceVolume>(record: &R, period: Duration) -> Self {
        let mut candle = Self {
            source: record.source(),
            channel: record.channel(),
            product: record.product(),
            period,
            open: f64::NAN,
            high: f64::NAN,
            low: f64::NAN,
            close: f64::NAN,
            volume: f64::NAN,
        };

        candle.update(record);
        candle
    }

    pub fn update<T: PriceVolume>(&mut self, data: &T) {
        // update only against valid data
        if !data.volume().is_normal() || data.price().is_nan() {
            return;
        }

        if self.open.is_nan() {
            self.open = data.price();
        }

        self.high = self.high.max(data.price());
        self.low = self.low.min(data.price());
        self.close = data.price();

        if self.volume.is_nan() {
            self.volume = data.volume();
        } else {
            self.volume += data.volume();
        }
    }

    pub fn is_nan(&self) -> bool {
        self.open.is_nan() || self.high.is_nan() || self.low.is_nan() || self.close.is_nan()
    }

    pub fn reset(&mut self) -> Self {
        let previous = (*self).clone();
        self.open = f64::NAN;
        self.high = f64::NAN;
        self.low = f64::NAN;
        self.close = f64::NAN;
        self.volume = f64::NAN;
        previous
    }
}

impl<R: PriceVolume> PartialEq<R> for Candle {
    fn eq(&self, other: &R) -> bool {
        self.source == other.source()
            && self.channel == other.channel()
            && self.product == other.product()
    }
}

impl OHLCV for Candle {
    #[inline]
    fn open(&self) -> ValueType {
        self.open
    }

    #[inline]
    fn high(&self) -> ValueType {
        self.high
    }

    #[inline]
    fn low(&self) -> ValueType {
        self.low
    }

    #[inline]
    fn close(&self) -> ValueType {
        self.close
    }

    #[inline]
    fn volume(&self) -> ValueType {
        self.volume
    }
}

impl TryInto<Post> for Candle {
    type Error = Error;

    fn try_into(self) -> Result<Post, Self::Error> {
        let tag = format!("candle_{}_{}_{}", self.source, self.channel, self.product,);
        log::debug!("Emit post with tag: {:?}", tag);

        Ok(Post::new(serde_json::to_value(self)?, HashMap::new(), tag))
    }
}

impl TryFrom<Post> for Candle {
    type Error = Error;

    fn try_from(value: Post) -> Result<Self, Self::Error> {
        let candle = serde_json::from_value::<Candle>(value.body)?;
        Ok(candle)
    }
}

impl PartialEq for Candle {
    fn eq(&self, other: &Self) -> bool {
        self.open.to_bits() == other.open().to_bits()
            && self.high.to_bits() == other.high().to_bits()
            && self.low.to_bits() == other.low().to_bits()
            && self.close.to_bits() == other.close().to_bits()
            && self.volume.to_bits() == other.volume().to_bits()
    }
}

impl Eq for Candle {}

#[cfg(test)]
mod tests {
    use super::Candle;
    use crate::record::PriceVolume;
    use rstest::*;
    use tokio::time::Duration;
    use yata::core::OHLCV;

    #[derive(Default, Debug)]
    struct Input {
        _price: f64,
        _volume: f64,
    }

    impl PriceVolume for Input {
        fn price(&self) -> f64 {
            self._price
        }

        fn volume(&self) -> f64 {
            self._volume
        }

        fn source(&self) -> String {
            "source".to_string()
        }

        fn channel(&self) -> String {
            "channel".to_string()
        }

        fn product(&self) -> String {
            "product".to_string()
        }
    }

    #[rstest]
    #[case::default(
        vec![Input{_price: 4.0, _volume: 1.0}],
        vec![4.0, 4.0, 4.0, 4.0, 1.0]
    )]
    #[case::simple(
        vec![Input{_price: 4.0, _volume: 1.0},Input{_price: 10.0, _volume: 1.0}, Input{_price: 1.0, _volume: 1.0}, Input{_price: 5.0, _volume: 1.0}],
        vec![4.0, 10.0, 1.0, 5.0, 4.0]
    )]
    #[case::novolume(
        vec![Input{_price: 4.0, _volume: 1.0},Input{_price: 10.0, _volume: f64::NAN}],
        vec![4.0, 4.0, 4.0, 4.0, 1.0]
    )]
    #[case::noprice(
        vec![Input{_price: 4.0, _volume: 1.0},Input{_price: f64::NAN, _volume: 1.0}],
        vec![4.0, 4.0, 4.0, 4.0, 1.0]
    )]
    fn test_has_changed(#[case] timeline: Vec<Input>, #[case] expected: Vec<f64>) {
        let mut candle = Candle::new(timeline.get(0).unwrap(), Duration::default());

        for input in timeline.iter().skip(1) {
            candle.update(input);
        }
        assert_eq!(&candle.open(), expected.get(0).unwrap());
        assert_eq!(&candle.high(), expected.get(1).unwrap());
        assert_eq!(&candle.low(), expected.get(2).unwrap());
        assert_eq!(&candle.close(), expected.get(3).unwrap());
        assert_eq!(&candle.volume(), expected.get(4).unwrap());
        candle.reset();
        assert!(candle.open().is_nan())
    }
}
