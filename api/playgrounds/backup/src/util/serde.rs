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

pub mod s3 {
    use super::*;

    pub mod object_lock_retention_mode {
        use ::s3::types::ObjectLockRetentionMode;

        use super::*;

        #[inline]
        pub fn deserialize<'de, D>(deserializer: D) -> Result<ObjectLockRetentionMode, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            match value.as_str() {
                "compliance" => Ok(ObjectLockRetentionMode::Compliance),
                "governance" => Ok(ObjectLockRetentionMode::Governance),
                _ => Err(serde::de::Error::custom(
                    "Unknown ObjectLockRetentionMode `{value}`. Allowed values: `\"compliance\"`, `\"governance\"`.",
                )),
            }
        }
    }

    pub mod object_lock_legal_hold_status {
        use ::s3::types::ObjectLockLegalHoldStatus;

        use super::*;

        #[inline]
        pub fn deserialize<'de, D>(deserializer: D) -> Result<ObjectLockLegalHoldStatus, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            match value.as_str() {
                "on" => Ok(ObjectLockLegalHoldStatus::On),
                "off" => Ok(ObjectLockLegalHoldStatus::Off),
                _ => Err(serde::de::Error::custom(
                    "Unknown ObjectLockLegalHoldStatus `{value}`. Allowed values: `\"on\"`, `\"off\"`.",
                )),
            }
        }

        pub mod option {
            use super::*;

            #[inline]
            pub fn deserialize<'de, D>(
                deserializer: D,
            ) -> Result<Option<ObjectLockLegalHoldStatus>, D::Error>
            where
                D: Deserializer<'de>,
            {
                super::deserialize(deserializer).map(Some)
            }
        }
    }
}
