// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::de;
use serde::{Deserialize, Deserializer};

pub mod iso8601_duration {
    use tokio::time::Duration;

    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let iso_duration = ::iso8601_duration::Duration::deserialize(deserializer)?;
        match iso_duration.to_std() {
            Some(std_duration) => Ok(std_duration),
            None => Err(de::Error::custom(
                "Duration cannot contain years or months (unquantifiable). \
                Use for example `P365D` or `P30D` to make your expected result explicit.",
            )),
        }
    }
}

pub mod iso8601_duration_or_secs {
    use tokio::time::Duration;

    use super::*;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Iso8601DurationOrSecs {
        #[serde(with = "super::iso8601_duration")]
        Iso8601(Duration),
        Secs(u64),
        SecsAsString(String),
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Iso8601DurationOrSecs::deserialize(deserializer) {
            Ok(Iso8601DurationOrSecs::Iso8601(duration)) => Ok(duration),
            Ok(Iso8601DurationOrSecs::Secs(secs)) => Ok(Duration::from_secs(secs)),
            Ok(Iso8601DurationOrSecs::SecsAsString(str)) => match str.parse::<u64>() {
                Ok(secs) => Ok(Duration::from_secs(secs)),
                Err(err) => {
                    tracing::debug!("Parsing error: {err:#}");
                    Err(de::Error::custom(
                        "Expected a duration (ISO 8601 duration or integer number of seconds).",
                    ))
                }
            },
            Err(err) => {
                tracing::debug!("Parsing error: {err:#}");
                Err(de::Error::custom(
                    "Expected a duration (ISO 8601 duration or integer number of seconds).",
                ))
            }
        }
    }

    pub mod option {
        use super::*;

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
        where
            D: Deserializer<'de>,
        {
            super::deserialize(deserializer).map(Some)
        }
    }
}

pub mod null_as_some_none {
    use super::*;

    /// Any value that is present is considered `Some` value, including `null`.
    ///
    /// Copyright: [Treat null and missing field as being different · Issue #984 · serde-rs/serde](https://github.com/serde-rs/serde/issues/984#issuecomment-314143738).
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer).map(Some)
    }
}

pub mod backup_config_opt {
    use super::*;

    /// Returns `None` instead of an error is backup storage not defined.
    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Option<prose_backup::BackupConfig>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match prose_backup::BackupConfig::deserialize(deserializer) {
            Ok(config) => Ok(Some(config)),
            Err(error) => {
                let err = error.to_string();
                if err.contains("missing field `storage`")
                    || err.contains("missing field `provider`")
                {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }
}
