/*
 * Copyright 2022 Google LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::sync::Arc;

use chrono::prelude::*;
use once_cell::sync::Lazy;
use prometheus::HistogramVec;

use crate::{
    filters::prelude::*,
    metadata::Value,
    metrics::{
        histogram_opts, registry, BUCKET_COUNT, BUCKET_FACTOR, BUCKET_START, DIRECTION_LABEL,
        METADATA_KEY_LABEL, READ_DIRECTION_LABEL, WRITE_DIRECTION_LABEL,
    },
};

crate::include_proto!("quilkin.filters.timestamp.v1alpha1");
use self::quilkin::filters::timestamp::v1alpha1 as proto;

pub(crate) static METRIC: Lazy<HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec_with_registry! {
        histogram_opts(
            "seconds",
            SUBSYSTEM,
            "The duration of seconds of the `metadata_key` metric",
            prometheus::exponential_buckets(BUCKET_START, BUCKET_FACTOR, BUCKET_COUNT).unwrap(),
        ),
        &[METADATA_KEY_LABEL, DIRECTION_LABEL],
        registry(),
    }
    .unwrap()
});

const SUBSYSTEM: &str = "filters_timestamp";

/// A filter that reads a metadata value as a timestamp to be observed in
/// a histogram.
#[derive(Debug, Clone)]
pub struct Timestamp {
    config: Arc<Config>,
}

impl Timestamp {
    /// Observes the duration since a timestamp stored in `metadata` and now,
    /// if present.
    pub fn observe(
        &self,
        metadata: &crate::metadata::DynamicMetadata,
        direction_label: &'static str,
    ) {
        let value = metadata
            .get(self.config.metadata_key)
            .and_then(|item| match item {
                Value::Number(item) => Some(*item as i64),
                Value::Bytes(vec) => Some(i64::from_be_bytes((**vec).try_into().ok()?)),
                _ => None,
            });

        let value = match value {
            Some(item) => item,
            None => return,
        };

        const BILLION: i64 = 1_000_000_000;
        let (secs, nsecs) = (value / BILLION, (value % BILLION) as u32);
        let naive = match NaiveDateTime::from_timestamp_opt(secs, nsecs) {
            Some(datetime) => datetime,
            None => {
                tracing::warn!(
                    timestamp = value,
                    metadata_key = %self.config.metadata_key,
                    "invalid unix timestamp"
                );
                return;
            }
        };

        // Create a normal DateTime from the NaiveDateTime
        let datetime: DateTime<Utc> = DateTime::from_utc(naive, Utc);

        let now = Utc::now();
        let seconds = now.signed_duration_since(datetime).num_seconds();
        self.metric(direction_label).observe(seconds as f64);
    }

    fn metric(&self, direction_label: &'static str) -> prometheus::Histogram {
        METRIC.with_label_values(&[&self.config.metadata_key.to_string(), direction_label])
    }
}

impl Timestamp {
    fn new(config: Config) -> Result<Self, Error> {
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl TryFrom<Config> for Timestamp {
    type Error = Error;

    fn try_from(config: Config) -> Result<Self, Self::Error> {
        Self::new(config)
    }
}

impl Filter for Timestamp {
    fn read(&self, ctx: &mut ReadContext) -> Option<()> {
        self.observe(&ctx.metadata, READ_DIRECTION_LABEL);
        Some(())
    }

    fn write(&self, ctx: &mut WriteContext) -> Option<()> {
        self.observe(&ctx.metadata, WRITE_DIRECTION_LABEL);
        Some(())
    }
}

impl StaticFilter for Timestamp {
    const NAME: &'static str = "quilkin.filters.timestamp.v1alpha1.Timestamp";
    type Configuration = Config;
    type BinaryConfiguration = proto::Timestamp;

    fn try_from_config(config: Option<Self::Configuration>) -> Result<Self, Error> {
        Self::new(Self::ensure_config_exists(config)?)
    }
}

/// Config represents a [self]'s configuration.
#[derive(
    Clone, Debug, Eq, PartialEq, schemars::JsonSchema, serde::Serialize, serde::Deserialize,
)]
pub struct Config {
    /// The metadata key to read the UTC UNIX Timestamp from.
    #[serde(rename = "metadataKey")]
    pub metadata_key: crate::metadata::Key,
}

impl Config {
    pub fn new(metadata_key: impl AsRef<str>) -> Self {
        Self {
            metadata_key: metadata_key.as_ref().into(),
        }
    }
}

impl From<Config> for proto::Timestamp {
    fn from(config: Config) -> Self {
        Self {
            metadata_key: Some(config.metadata_key.to_string()),
        }
    }
}

impl TryFrom<proto::Timestamp> for Config {
    type Error = ConvertProtoConfigError;

    fn try_from(p: proto::Timestamp) -> Result<Self, Self::Error> {
        p.metadata_key
            .map(Self::new)
            .ok_or_else(|| ConvertProtoConfigError::missing_field("metadata_key"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::filters::{capture, concatenate, Capture, Concatenate};

    #[test]
    fn basic() {
        const TIMESTAMP_KEY: &str = "BASIC";
        let filter = Timestamp::from_config(Config::new(TIMESTAMP_KEY).into());
        let mut ctx = ReadContext::new(
            vec![],
            (std::net::Ipv4Addr::UNSPECIFIED, 0).into(),
            b"hello".to_vec(),
        );
        ctx.metadata.insert(
            TIMESTAMP_KEY.into(),
            Value::Number(Utc::now().timestamp() as u64),
        );

        filter.read(&mut ctx).unwrap();

        assert_eq!(1, filter.metric(READ_DIRECTION_LABEL).get_sample_count());
    }

    #[test]
    fn with_capture_and_timestamp() {
        const TIMESTAMP_KEY: &str = "WITH_CAPTURE";
        let concat = Concatenate::from_config(
            concatenate::Config {
                on_read: concatenate::Strategy::Append,
                on_write: <_>::default(),
                value: crate::metadata::Symbol::reference("quilkin.dev/computed/timestamp/now"),
            }
            .into(),
        );
        let capture = Capture::from_config(
            capture::Config {
                metadata_key: TIMESTAMP_KEY.into(),
                strategy: capture::Suffix {
                    remove: true,
                    size: 8,
                }
                .into(),
            }
            .into(),
        );
        let timestamp = Timestamp::from_config(Config::new(TIMESTAMP_KEY).into());
        let source = (std::net::Ipv4Addr::UNSPECIFIED, 0);
        let mut ctx = ReadContext::new(vec![], source.into(), Vec::new());

        concat.read(&mut ctx).unwrap();
        capture.read(dbg!(&mut ctx)).unwrap();
        timestamp.read(&mut ctx).unwrap();

        assert_eq!(1, timestamp.metric(READ_DIRECTION_LABEL).get_sample_count());
    }
}
