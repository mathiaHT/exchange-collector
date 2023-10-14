#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Serde failed: {}", .source)]
    SerdeJson {
        #[from]
        source: serde_json::Error,
    },
    #[error("Serde failed: {}", .source)]
    SerdeDynamo {
        #[from]
        source: serde_dynamo::Error,
    },
    #[error("Deserilaze failed: {}", .source)]
    SerdeYaml {
        #[from]
        source: serde_yaml::Error,
    },
    #[error("Failed to parse url: {}", .source)]
    Url {
        #[from]
        source: url::ParseError,
    },
    #[error("Establish connection failed: {}", .source)]
    Tungstenite {
        #[from]
        source: tokio_tungstenite::tungstenite::error::Error,
    },
    #[error("Returned http status is failure.")]
    Http(String),
    #[error("Htt request failed: {}", .source)]
    Request {
        #[from]
        source: reqwest::Error,
    },
    /// Error sending websocket message
    #[error("Send failed")]
    Send(#[source] tokio_tungstenite::tungstenite::Error),
    /// Error reading from websocket
    #[error("Read failed")]
    Read(#[source] tokio_tungstenite::tungstenite::Error),
    /// Error returned when exporter exit timeout
    #[error("Failed to send timer event: {}", .source)]
    Timer {
        #[from]
        source: tokio::sync::broadcast::error::SendError<crate::channel::Post>,
    },
    #[error("Exit actor timeout: {}", .source)]
    Timeout {
        #[from]
        source: tokio::time::error::Elapsed,
    },
    #[error("Failed to batch write items: {}", .source)]
    DynamoDBBatchWrite {
        #[from]
        source: rusoto_core::RusotoError<rusoto_dynamodb::BatchWriteItemError>,
    },
    #[error("Failed to put items: {}", .source)]
    DynamoDBPutItem {
        #[from]
        source: rusoto_core::RusotoError<rusoto_dynamodb::PutItemError>,
    },
    #[error("Failed to parse region: {}", .source)]
    AwsParseRegion {
        #[from]
        source: rusoto_core::region::ParseRegionError,
    },
    #[error("Failed to receive: {}", .source)]
    Receive {
        #[from]
        source: tokio::sync::broadcast::error::RecvError,
    },
    #[error("Failed to init indicator: {}", .source)]
    Yata {
        #[from]
        source: yata::core::Error,
    },
    #[error("Unexpected end of stream")]
    UnexpectedEndOfStream,
    #[error("Expected a tungstenite::Message")]
    ExpectedTungsteniteMessage,
    #[error("Missing field `source`.")]
    SourceNotFound(String),
    #[error("Post is not a map.")]
    PostNotValidMap,
    /// Error return by the crate opentelemetry
    #[error("Metrics failed: {}", .source)]
    OpenTelemetry {
        #[from]
        source: opentelemetry::metrics::MetricsError,
    },
    /// Error returned when the table failed to create
    #[error("Failed to get backend: {}", .source)]
    Delta {
        #[from]
        source: deltalake::DeltaTableError,
    },
    /// Error returned when building indicators
    #[error("Failed to build config")]
    Config(String),
    #[error("Strum failed: {}", .source)]
    Parse {
        #[from]
        source: strum::ParseError,
    },
    // Error returned when channel is not setup
    #[error("Setup is not complete")]
    Setup,
    #[error("IO failure: {}", .source)]
    IO {
        #[from]
        source: std::io::Error,
    },
    #[error("Cannot process event")]
    UnprocessableEvent(String),
    #[error("Failed to get s3 object: {}", .source)]
    S3 {
        #[from]
        source: rusoto_core::RusotoError<rusoto_s3::GetObjectError>,
    },
    #[error("Internal server error")]
    InternalServerError(String),
    #[error("Arrow failed: {}", .source)]
    Arrow {
        #[from]
        source: arrow::error::ArrowError,
    },
    #[error("DeltaArrow failed: {}", .source)]
    DeltaArrow {
        #[from]
        source: deltalake::arrow::error::ArrowError,
    },
}
