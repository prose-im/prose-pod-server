// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::{Deserialize, Deserializer};

pub mod iso8601_duration {
    use tokio::time::Duration;

    use serde::de;

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
