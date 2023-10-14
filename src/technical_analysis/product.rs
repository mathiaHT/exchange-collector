use enum_dispatch::enum_dispatch;
use opentelemetry::{
    metrics::{Meter, ObservableGauge},
    KeyValue,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::Duration;
use yata::{
    core::{IndicatorConfig, IndicatorResult, OHLCV},
    indicators::*,
};

use super::candle::Candle;
use super::dd::{IndicatorConfigDyn, IndicatorInstanceDyn};
use crate::{
    error::Error,
    metrics::{instrument, metadata},
};

pub const NEUTRAL_SIGNAL: f64 = 0.0;

/// Expose methods of external trait [`IndicatorConfig`] with extra functionnalities.
///
/// This trait will be implemented for every struct that implemented [`IndicatorConfig`] trait.
/// This trait allows dynamic dispatching with enum_dispatch
#[enum_dispatch]
pub trait WithIndicatorConfig: Send + Sync {
    fn dyn_config<T: OHLCV>(&self) -> Box<dyn IndicatorConfigDyn<T> + Send + Sync>;

    fn name(&self) -> &'static str;

    fn validate_config(&self) -> bool;
}

impl<I> WithIndicatorConfig for I
where
    I: IndicatorConfig + Send + Sync + 'static,
    <I>::Instance: Send + Sync,
{
    fn dyn_config<T: OHLCV>(&self) -> Box<dyn IndicatorConfigDyn<T> + Send + Sync> {
        Box::new(self.clone())
    }

    fn name(&self) -> &'static str {
        self.name()
    }

    fn validate_config(&self) -> bool {
        self.validate()
    }
}

#[enum_dispatch(IndicatorConfig, WithIndicatorConfig)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Indicator {
    #[serde(rename = "aroon")]
    Aroon(Aroon),
    #[serde(rename = "average_directional_index")]
    AverageDirectionalIndex(AverageDirectionalIndex),
    #[serde(rename = "awesome_oscillator")]
    AwesomeOscillator(AwesomeOscillator),
    #[serde(rename = "bollinger_bands")]
    BollingerBands(BollingerBands),
    #[serde(rename = "chainkin_money_flow")]
    ChaikinMoneyFlow(ChaikinMoneyFlow),
    #[serde(rename = "chainkin_oscillator")]
    ChaikinOscillator(ChaikinOscillator),
    #[serde(rename = "chande_kroll_stop")]
    ChandeKrollStop(ChandeKrollStop),
    #[serde(rename = "chande_momentum_oscillator")]
    ChandeMomentumOscillator(ChandeMomentumOscillator),
    #[serde(rename = "commodity_channel_index")]
    CommodityChannelIndex(CommodityChannelIndex),
    #[serde(rename = "coppock_curve")]
    CoppockCurve(CoppockCurve),
    #[serde(rename = "detrended_price_oscillator")]
    DetrendedPriceOscillator(DetrendedPriceOscillator),
    #[serde(rename = "donchian_channel")]
    DonchianChannel(DonchianChannel),
    #[serde(rename = "ease_of_movement")]
    EaseOfMovement(EaseOfMovement),
    #[serde(rename = "elders_force_index")]
    EldersForceIndex(EldersForceIndex),
    #[serde(rename = "envelopes")]
    Envelopes(Envelopes),
    #[serde(rename = "fisher_transform")]
    FisherTransform(FisherTransform),
    #[serde(rename = "hull_moving_average")]
    HullMovingAverage(HullMovingAverage),
    #[serde(rename = "ichimoku_cloud")]
    IchimokuCloud(IchimokuCloud),
    #[serde(rename = "kaufman")]
    Kaufman(Kaufman),
    #[serde(rename = "keltner_channel")]
    KeltnerChannel(KeltnerChannel),
    #[serde(rename = "klinger_volume_oscillator")]
    KlingerVolumeOscillator(KlingerVolumeOscillator),
    #[serde(rename = "know_sure_thing")]
    KnowSureThing(KnowSureThing),
    #[serde(rename = "macd")]
    Macd(MACD),
    #[serde(rename = "momentum_index")]
    MomentumIndex(MomentumIndex),
    #[serde(rename = "money_flow_index")]
    MoneyFlowIndex(MoneyFlowIndex),
    #[serde(rename = "parabolic_sar")]
    ParabolicSAR(ParabolicSAR),
    #[serde(rename = "pivot_reversal_strategy")]
    PivotReversalStrategy(PivotReversalStrategy),
    #[serde(rename = "price_channel_strategy")]
    PriceChannelStrategy(PriceChannelStrategy),
    #[serde(rename = "relative_strength_index")]
    RelativeStrengthIndex(RelativeStrengthIndex),
    #[serde(rename = "relative_vigor_index")]
    RelativeVigorIndex(RelativeVigorIndex),
    #[serde(rename = "smi_ergodic_indicator")]
    SMIErgodicIndicator(SMIErgodicIndicator),
    #[serde(rename = "stochastic_oscillator")]
    StochasticOscillator(StochasticOscillator),
    #[serde(rename = "trend_strength_index")]
    TrendStrengthIndex(TrendStrengthIndex),
    #[serde(rename = "trix")]
    Trix(Trix),
    #[serde(rename = "true_strength_index")]
    TrueStrengthIndex(TrueStrengthIndex),
    #[serde(rename = "woodies_cci")]
    WoodiesCCI(WoodiesCCI),
}

pub struct ProductInstance {
    source: String,
    channel: String,
    product: String,
    period: Duration,
    config: Box<dyn IndicatorConfigDyn<Candle>>,
    instance: Option<Box<dyn IndicatorInstanceDyn<Candle>>>,
    value_metrics: Vec<ObservableGauge<f64>>,
    signal_metrics: Vec<ObservableGauge<f64>>,
    metadata: [KeyValue; 5],
}

impl ProductInstance {
    pub fn try_new(candle: &Candle, config: &Indicator, meter: Arc<Meter>) -> Result<Self, Error> {
        let meter = meter;

        let config = config.dyn_config::<Candle>();
        let name = config.name().to_lowercase();

        let (value_length, signal_length) = config.size();
        let mut signal_metrics: Vec<ObservableGauge<f64>> = vec![];
        let mut value_metrics: Vec<ObservableGauge<f64>> = vec![];

        let metadata = [
            KeyValue::new(metadata::INDICATOR, name),
            KeyValue::new(metadata::PRODUCT, candle.product.to_lowercase()),
            KeyValue::new(metadata::SOURCE, candle.source.to_lowercase()),
            KeyValue::new(metadata::CHANNEL, candle.channel.to_lowercase()),
            KeyValue::new(metadata::CANDLE_PERIOD, candle.period.as_secs().to_string()),
        ];

        for _ in 0..signal_length {
            signal_metrics.push(
                meter
                    .f64_observable_gauge(instrument::INDICATOR_SIGNAL)
                    .try_init()?,
            )
        }

        for _ in 0..value_length {
            value_metrics.push(
                meter
                    .f64_observable_gauge(instrument::INDICATOR_VALUE)
                    .try_init()?,
            )
        }

        Ok(Self {
            source: candle.source.to_string(),
            channel: candle.channel.to_string(),
            product: candle.product.to_string(),
            period: candle.period,
            config,
            instance: None,
            signal_metrics,
            value_metrics,
            metadata,
        })
    }

    fn init(&mut self, candle: &Candle) -> Result<(), Error> {
        self.instance.replace(self.config.init(candle)?);
        Ok(())
    }

    /// Process if possible next indicator value and update metrics
    pub fn next(&mut self, candle: &Candle) -> Result<Option<IndicatorResult>, Error> {
        if let Some(mut instance) = self.instance.take() {
            let result = instance.next(candle);
            self.record(&result);
            self.instance.replace(instance);

            Ok(Some(result))
        } else {
            self.init(candle)?;
            Ok(None)
        }
    }

    pub fn reset(&mut self) {
        self.instance = None;
    }

    fn record(&self, result: &IndicatorResult) {
        for (idx, metric) in self.value_metrics.iter().enumerate() {
            let mut metadata = self.metadata.clone().to_vec();
            metadata.push(KeyValue::new(metadata::VALUE, idx.to_string()));
            metric.observe(result.value(idx), &metadata[..]);
        }

        for (idx, metric) in self.signal_metrics.iter().enumerate() {
            let mut metadata = self.metadata.clone().to_vec();
            metadata.push(KeyValue::new(metadata::SIGNAL, idx.to_string()));
            metric.observe(
                result.signal(idx).ratio().unwrap_or(NEUTRAL_SIGNAL),
                &metadata[..],
            );
        }
    }
}

impl PartialEq<Candle> for ProductInstance {
    fn eq(&self, other: &Candle) -> bool {
        self.source == other.source
            && self.channel == other.channel
            && self.product == other.product
            && self.period == other.period
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::technical_analysis::dd::IndicatorConfigDyn;
    // use opentelemetry::global;
    use rstest::*;
    // use std::sync::Arc;
    use yata::{core::Candle, indicators::MACD};

    #[fixture]
    fn config() -> Box<dyn IndicatorConfigDyn<Candle>> {
        Box::new(MACD::default())
    }

    #[rstest]
    #[case::aroon(
        r#"
        {
            "type": "aroon",
            "period": 14,
            "signal_zone": 0.3,
            "over_zone_period": 7
        }
        "#,
        "Aroon"
    )]
    #[case::average_directional_index(
        r#"
        {
            "type": "average_directional_index",
            "period1": 1,
            "zone": 0.2,
            "method1": {
                "rma": 11
            },
            "method2": {
                "rma": 12
            }
        }
        "#,
        "AverageDirectionalIndex"
    )]
    #[case::macd(
        r#"
        {
            "type": "macd",
            "ma1": {
                "ema": 12
            },
            "ma2": {
                "ema": 26
            },
            "signal": {
                "ema": 9
            },
            "source": "close"
        }
        "#,
        "MACD"
    )]
    fn test_indicator(#[case] config: &'static str, #[case] expected: &'static str) {
        let indicator = serde_json::from_str::<Indicator>(config).unwrap();
        assert_eq!(expected, indicator.dyn_config::<Candle>().name());
        assert!(indicator.validate_config())
    }
}
