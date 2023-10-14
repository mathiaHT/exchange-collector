//! Connectors is a crate that implements structures to help
//! * export exchange data
//! * receive exchange data
//!
//! ## Inspiration
//!
//! * Inspiration from [openteleletry collector](https://github.com/open-telemetry/opentelemetry-collector)
//! * use [green threads](https://en.wikipedia.org/wiki/Green_threads)
//! * event oriented using queues to exchange data between connectors
//!
//! ## Features
//!
//! This crates can be seen as a hierarchy of connectors. Therefore:
//! * Each connector type (regardless it is a receiver or exporter) is a feature,
//! it activates the dependencies to manage the behaviour of this connector type
//! but not the dependencies of the implementors.
//! * Each specific connector (e.g. exporter::datalake::deltalake) is a sub feature.
//! It activates the both connector type and connector specific dependencies

#[macro_use]
extern crate lazy_static;
extern crate log;
extern crate strum;
extern crate strum_macros;

mod actor;
mod channel;
mod error;
mod record;
mod utils;

mod exporter;
mod metrics;
mod processor;
mod websocket;

mod matches;

mod delta;
mod exchange;
mod technical_analysis;

mod pipeline;

use async_trait::async_trait;
use chrono::Local;
use enum_dispatch::enum_dispatch;
use env_logger::Builder;
use error::Error;
use log::LevelFilter;
use metrics::MetricsExporterBuilder;
use opentelemetry::metrics::MeterProvider;
use pipeline::PipelineConfig;
use rusoto_core::Region;
use std::{borrow::Cow, io::Write, path::PathBuf, str::FromStr, sync::Arc};
use structopt::StructOpt;

const AWS_ENDPOINT_URL: &str = "AWS_ENDPOINT_URL";
const AWS_REGION: &str = "AWS_REGION";
const AWS_DEFAULT_REGION: &str = "us-east-1";
const LOG_LEVEL: &str = "LOG_LEVEL";
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "listener", about = "Stock agent cli.")]
struct MainCommand {
    #[structopt(
        long = "log-level",
        help = "Configuration path for the market indicators.",
        env = LOG_LEVEL,
        default_value = "info"
    )]
    pub log_level: LevelFilter,
    #[structopt(subcommand)]
    command: Command,
}

#[async_trait]
#[enum_dispatch]
trait Execute {
    async fn execute(&self) -> Result<(), Error>;
}

#[derive(Clone, Debug, StructOpt)]
#[enum_dispatch(Execute)]
enum Command {
    Run(RunCommand),
    Validate(ValidateCommand),
}

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "validate", about = "Run an agent with a config.")]
struct ValidateCommand {
    #[structopt(
        long = "config",
        short = "c",
        help = "Path to the configuration to validate."
    )]
    pub configuration_path: PathBuf,
    #[structopt(long = "show", help = "Path to the configuration to validate.")]
    pub show: bool,
    #[structopt(
        short = "r",
        long = "region",
        env = AWS_REGION,
        help = "Bucket region for the configuration.",
        default_value = AWS_DEFAULT_REGION
    )]
    pub region: String,
    #[structopt(
        short = "u",
        long = "endpoint",
        env = AWS_ENDPOINT_URL,
        help = "AWS endpoint."
    )]
    pub endpoint: Option<String>,
}

#[async_trait]
impl Execute for ValidateCommand {
    async fn execute(&self) -> Result<(), Error> {
        let region = if let Some(endpoint) = self.endpoint.clone() {
            Region::Custom {
                name: self.region.clone(),
                endpoint,
            }
        } else {
            Region::from_str(self.region.as_str()).unwrap_or(Region::from_str(AWS_DEFAULT_REGION)?)
        };

        let config = PipelineConfig::try_from(self.configuration_path.clone(), region).await?;
        if self.show {
            log::info!("{:#?}", config);
        };
        Ok(())
    }
}

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "run", about = "Run an agent with a config.")]
struct RunCommand {
    #[structopt(
        short = "c",
        long = "config",
        help = "Configuration path for the market indicators."
    )]
    pub configuration_path: PathBuf,
    #[structopt(
        short = "r",
        long = "region",
        env = AWS_REGION,
        help = "Bucket region for the configuration.",
        default_value = AWS_DEFAULT_REGION
    )]
    pub region: String,
    #[structopt(
        short = "u",
        long = "endpoint",
        env = AWS_ENDPOINT_URL,
        help = "AWS endpoint."
    )]
    pub endpoint: Option<String>,
}

#[async_trait]
impl Execute for RunCommand {
    async fn execute(&self) -> Result<(), Error> {
        log::info!("Start {}!", PKG_NAME);

        let region = if let Some(endpoint) = self.endpoint.clone() {
            Region::Custom {
                name: self.region.clone(),
                endpoint,
            }
        } else {
            Region::from_str(self.region.as_str()).unwrap_or(Region::from_str(AWS_DEFAULT_REGION)?)
        };

        // Read configuration from file
        let pipeline_configuration =
            PipelineConfig::try_from(self.configuration_path.clone(), region).await?;

        // Initialize opentelemetry meter
        let otel_controller = pipeline_configuration.service.metrics.build()?;
        let otel_meter = Arc::new(otel_controller.versioned_meter(
            PKG_NAME,
            Some(PKG_VERSION),
            None::<Cow<'static, str>>,
            None,
        ));

        // Initialize pipeline
        let (_pipeline, handles) = pipeline_configuration.init(otel_meter.clone()).await?;

        tokio::select! {
            result = handles => {
            log::info!("Actors have stopped: {:?}", result)
            },
        }
        Ok(())
    }
}

#[tokio::main]
async fn run_app() -> Result<(), Error> {
    let opt = MainCommand::from_args();
    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} - {} - {} - {}",
                Local::now().format("%Y-%m-%dT%H:%M:%S"),
                record.target(),
                record.level(),
                record.args()
            )
        })
        .filter(None, opt.log_level)
        .init();
    opt.command.execute().await
}

fn main() {
    std::process::exit(match run_app() {
        Ok(_) => 0,
        Err(err) => {
            log::error!("error: {:?}", err);
            1
        }
    });
}
