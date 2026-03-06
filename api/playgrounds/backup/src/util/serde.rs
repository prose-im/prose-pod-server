// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::{Deserialize as _, Deserializer};

/// `std::time::Duration` in [ISO 8601 Duration format](https://en.wikipedia.org/wiki/ISO_8601#Durations).
pub mod iso8601_duration {
    use std::time::Duration;

    use super::*;

    #[inline]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let duration = ::iso8601_duration::Duration::deserialize(deserializer)?;
        duration.to_std().ok_or(serde::de::Error::custom(
            "Duration contains years or months.",
        ))
    }
}
