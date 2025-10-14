// prose-pod-server-api
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
