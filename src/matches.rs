use crate::{
    error::Error,
    record::{PriceVolume, Record},
    utils::serde::f64_from_string,
};
use chrono::{DateTime, Utc};
use deltalake::{
    arrow::{
        array::{Array, Float64Array, StringArray, TimestampMicrosecondArray},
        record_batch::RecordBatch,
    },
    open_table_with_storage_options,
    operations::create::CreateBuilder,
    table::builder::s3_storage_options,
    DeltaTable, DeltaTableError, Schema, SchemaDataType, SchemaField, SchemaTypeStruct,
};
use dynamodb_lock::dynamo_lock_options;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, sync::Arc};

const MATCH: &str = "matches";
pub const TABLE_NAME: &str = "matches";
const LOCK_PROVIDER: &str = "dynamodb";

lazy_static! {
    static ref MATCH_SCHEMA: Schema = {
        let price: SchemaField = SchemaField::new(
            String::from("price"),
            SchemaDataType::primitive(String::from("double")),
            false,
            HashMap::new(),
        );
        let volume: SchemaField = SchemaField::new(
            String::from("volume"),
            SchemaDataType::primitive(String::from("double")),
            false,
            HashMap::new(),
        );
        let product: SchemaField = SchemaField::new(
            String::from("product"),
            SchemaDataType::primitive(String::from("string")),
            false,
            HashMap::new(),
        );
        let source: SchemaField = SchemaField::new(
            String::from("source"),
            SchemaDataType::primitive(String::from("string")),
            false,
            HashMap::new(),
        );
        let time: SchemaField = SchemaField::new(
            String::from("time"),
            SchemaDataType::primitive(String::from("timestamp")),
            false,
            HashMap::new(),
        );
        let buyer: SchemaField = SchemaField::new(
            String::from("buyer"),
            SchemaDataType::primitive(String::from("string")), // uuid
            true,
            HashMap::new(),
        );
        let seller: SchemaField = SchemaField::new(
            String::from("seller"),
            SchemaDataType::primitive(String::from("string")), // uuid
            true,
            HashMap::new(),
        );

        SchemaTypeStruct::new(vec![
            price,
            volume,
            product,
            source,
            time,
            seller,
            buyer,
        ])
    };
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchRecord {
    #[serde(deserialize_with = "f64_from_string")]
    pub price: f64,
    #[serde(alias = "size", deserialize_with = "f64_from_string")]
    pub volume: f64,
    #[serde(alias = "product_id")]
    pub product: String,
    pub source: String,
    pub time: DateTime<Utc>,
    pub buyer_id: Option<String>,
    pub seller_id: Option<String>,
}

impl PriceVolume for MatchRecord {
    fn price(&self) -> f64 {
        self.price
    }

    fn volume(&self) -> f64 {
        self.volume
    }

    fn product(&self) -> String {
        self.product.to_string()
    }

    fn channel(&self) -> String {
        MATCH.to_string()
    }

    fn source(&self) -> String {
        self.source.to_string()
    }
}

pub fn build_matches_batch(chunk: Vec<Record>) -> Result<RecordBatch, Error> {
    let arrow_schema =
        <deltalake::arrow::datatypes::Schema as TryFrom<&Schema>>::try_from(&MATCH_SCHEMA)?;
    let arrow_schema_ref = Arc::new(arrow_schema);

    let mut prices = vec![];
    let mut volumes = vec![];
    let mut products = vec![];
    let mut sources = vec![];
    let mut times: Vec<i64> = vec![];
    let mut buyers = vec![];
    let mut sellers = vec![];

    for record in chunk.into_iter() {
        match record {
            Record::Match(value) => {
                prices.push(value.price);
                volumes.push(value.volume);
                products.push(value.product);
                sources.push(value.source);
                times.push(value.time.timestamp_subsec_micros().into());
                buyers.push(value.buyer_id);
                sellers.push(value.seller_id);
            }
        }
    }

    let arrow_array: Vec<Arc<dyn Array>> = vec![
        Arc::new(Float64Array::from(prices)),
        Arc::new(Float64Array::from(volumes)),
        Arc::new(StringArray::from(products)),
        Arc::new(StringArray::from(sources)),
        Arc::new(TimestampMicrosecondArray::from(times)),
        Arc::new(StringArray::from(buyers)),
        Arc::new(StringArray::from(sellers)),
    ];

    Ok(RecordBatch::try_new(arrow_schema_ref, arrow_array)?)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MatchTable {}

impl MatchTable {
    pub async fn get_or_create(
        bucket: String,
        prefix: String,
        lock_table: String,
        region: String,
        endpoint: Option<String>,
    ) -> Result<DeltaTable, Error> {
        let table_uri = format!("s3://{}/{}/{}", bucket, prefix, TABLE_NAME);
        let mut storage_options = HashMap::from([
            (
                s3_storage_options::AWS_S3_LOCKING_PROVIDER.to_string(),
                String::from(LOCK_PROVIDER),
            ),
            (s3_storage_options::AWS_REGION.to_string(), region.clone()),
            (
                dynamo_lock_options::DYNAMO_LOCK_TABLE_NAME.to_string(),
                lock_table.clone(),
            ),
            (
                dynamo_lock_options::DYNAMO_LOCK_PARTITION_KEY_VALUE.to_string(),
                table_uri.clone(),
            ),
        ]);

        if let Some(endpoint) = endpoint {
            storage_options.insert(s3_storage_options::AWS_ENDPOINT_URL.to_string(), endpoint);
        };

        if let Ok(value) = env::var("AWS_ALLOW_HTTP") {
            storage_options.insert(s3_storage_options::AWS_ALLOW_HTTP.to_string(), value);
        }

        match open_table_with_storage_options(table_uri.as_str(), storage_options.clone()).await {
            Ok(table) => Ok(table),
            Err(error) => match error {
                DeltaTableError::NotATable(_) => CreateBuilder::new()
                    .with_table_name(TABLE_NAME)
                    .with_location(table_uri)
                    .with_columns(MATCH_SCHEMA.get_fields().clone())
                    .with_storage_options(storage_options)
                    .await
                    .map_err(Error::from),
                _ => Err(Error::from(error)),
            },
        }
    }
}
