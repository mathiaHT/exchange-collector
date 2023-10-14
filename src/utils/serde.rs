//! Serde utils

use chrono::Utc;
use regex::Regex;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

struct F64InQuotes;

impl<'de> Visitor<'de> for F64InQuotes {
    type Value = f64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("f64 as a number or string")
    }

    fn visit_f64<E>(self, id: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(id)
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        s.parse().map_err(de::Error::custom)
    }
}

pub fn f64_from_string<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    d.deserialize_any(F64InQuotes)
}

pub fn f64_opt_from_string<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    d.deserialize_any(F64InQuotes).map(Some).or(Ok(None))
}

struct I64InQuotes;

impl<'de> Visitor<'de> for I64InQuotes {
    type Value = i64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("i64 as a number or string")
    }

    fn visit_i64<E>(self, id: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(id)
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        s.parse().map_err(de::Error::custom)
    }
}

pub fn i64_from_string<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    d.deserialize_any(I64InQuotes)
}

pub fn uuid_from_string<'de, D>(d: D) -> Result<Uuid, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Uuid::from_str(&s).map_err(de::Error::custom)
}

pub fn regex_opt_from_string<'de, D>(d: D) -> Result<Option<Regex>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    if s.is_empty() {
        Ok(None)
    } else {
        Regex::from_str(&s).map_err(de::Error::custom).map(Some)
    }
}

pub fn uuid_opt_from_string<'de, D>(d: D) -> Result<Option<Uuid>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    if s.is_empty() {
        Ok(None)
    } else {
        Uuid::from_str(&s).map_err(de::Error::custom).map(Some)
    }
}

#[allow(dead_code)]
pub fn f64_nan_from_string<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    d.deserialize_any(F64InQuotes).or(Ok(std::f64::NAN)) // not sure that 100% correct
}

struct UsizeInQuotes;

impl<'de> Visitor<'de> for UsizeInQuotes {
    type Value = usize;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("usize as a number or string")
    }

    fn visit_u64<E>(self, id: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(id as usize)
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        s.parse().map_err(de::Error::custom)
    }
}

#[allow(dead_code)]
pub fn usize_from_string<'de, D>(d: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    d.deserialize_any(UsizeInQuotes)
}

#[allow(dead_code)]
pub fn datetime_from_string<'de, D>(d: D) -> Result<chrono::DateTime<chrono::Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    (s + "").parse().map_err(de::Error::custom)
}

pub fn datetime_with_tz_from_string<'de, D>(d: D) -> Result<chrono::DateTime<chrono::Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    const FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f%#z";
    let s = String::deserialize(d)?;
    match chrono::DateTime::parse_from_str(&s, FORMAT).map_err(de::Error::custom) {
        Ok(dt) => Ok(dt.with_timezone(&Utc)),
        Err(err) => Err(err),
    }
}

#[allow(dead_code)]
pub fn option_datetime_with_tz_from_string<'de, D>(
    d: D,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper(
        #[serde(deserialize_with = "datetime_with_tz_from_string")] chrono::DateTime<chrono::Utc>,
    );

    let v = Option::deserialize(d)?;
    Ok(v.map(|Wrapper(a)| a))
}
