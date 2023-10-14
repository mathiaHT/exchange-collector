use super::error::Error;
use enum_dispatch::enum_dispatch;
use opentelemetry::runtime;
use opentelemetry_otlp::{ExportConfig, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::MeterProvider;
use serde::Deserialize;
use std::time::Duration;

pub mod instrument {
    pub const CANDLE_OPEN: &str = "open";
    pub const CANDLE_HIGH: &str = "high";
    pub const CANDLE_LOW: &str = "low";
    pub const CANDLE_CLOSE: &str = "close";
    pub const CANDLE_VOLUME: &str = "volume";
    pub const FLUSH_DURATION: &str = "flush_duration";
    pub const FLUSH_FAILED: &str = "flush_failed";
    pub const FLUSH_SUCCESS: &str = "flush_success";
    pub const HANDLE_FAILED: &str = "handle_failed";
    pub const HANDLE_SUCCESS: &str = "handle_success";
    pub const INDICATOR_SIGNAL: &str = "indicator_signal";
    pub const INDICATOR_VALUE: &str = "indicator_value";
    pub const INIT_COUNT: &str = "init_count";
    pub const NOT_SUPPORTED: &str = "not_supported";
    pub const RECEIVE_COUNT: &str = "receive_count";
    pub const SEND_FAILED: &str = "send_failed";
    pub const SEND_SUCCESS: &str = "send_success";
    pub const START_STREAM: &str = "send_success";
    pub const TASK_COUNT: &str = "task_count";
}

pub mod unit {
    pub const SECONDS: &str = "seconds";
    pub const MESSAGE: &str = "message";
    pub const TASK: &str = "task";
}

pub mod metadata {
    pub const ACTOR_NAME: &str = "actor_name";
    pub const CANDLE_PERIOD: &str = "candle_period";
    pub const CHANNEL: &str = "channel";
    pub const INDICATOR: &str = "indicator";
    pub const PRODUCT: &str = "product";
    pub const SIGNAL: &str = "signal";
    pub const SOURCE: &str = "source";
    pub const VALUE: &str = "value";
}

pub const EXPORTER_DEFAULT_INTERVAL: u64 = 15;
pub const EXPORTER_DEFAULT_ENDPOINT: &str = "http://localhost:4317";

#[enum_dispatch]
pub trait MetricsExporterBuilder {
    fn build(&self) -> Result<MeterProvider, Error>;
}

#[derive(Clone, Debug, Deserialize)]
pub struct OtlpConfig {
    pub period: Duration,
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            period: Duration::new(EXPORTER_DEFAULT_INTERVAL, 0),
            endpoint: None,
        }
    }
}

impl MetricsExporterBuilder for OtlpConfig {
    fn build(&self) -> Result<MeterProvider, Error> {
        let endpoint = self
            .clone()
            .endpoint
            .unwrap_or_else(|| String::from(EXPORTER_DEFAULT_ENDPOINT));

        let exporter_config = ExportConfig {
            endpoint,
            protocol: Protocol::Grpc,
            ..ExportConfig::default()
        };

        Ok(opentelemetry_otlp::new_pipeline()
            .metrics(runtime::Tokio)
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_export_config(exporter_config),
            )
            .with_period(self.period)
            .build()?)
    }
}

#[enum_dispatch(MetricsExporterBuilder)]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum MetricsConfig {
    #[serde(rename = "otlp")]
    Otlp(OtlpConfig),
}

impl Default for MetricsConfig {
    fn default() -> Self {
        MetricsConfig::Otlp(OtlpConfig::default())
    }
}
