// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::{Deserialize as _, Deserializer, de};

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

#[cfg(feature = "storage-s3")]
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
            match value.to_ascii_lowercase().as_str() {
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
            match value.to_ascii_lowercase().as_str() {
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

pub mod pgp {
    use super::*;

    pub mod passphrases {
        use std::collections::HashMap;

        use super::*;

        pub fn deserialize<'de, D>(
            deserializer: D,
        ) -> Result<HashMap<openpgp::Fingerprint, openpgp::crypto::Password>, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct V;

            impl<'de> de::Visitor<'de> for V {
                type Value = HashMap<openpgp::Fingerprint, openpgp::crypto::Password>;

                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a map of fingerprint strings to passphrase strings")
                }

                fn visit_map<A: de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut out = HashMap::new();
                    while let Some((k, v)) = map.next_entry::<String, String>()? {
                        let fingerprint = k
                            .parse::<openpgp::Fingerprint>()
                            .map_err(de::Error::custom)?;
                        out.insert(fingerprint, openpgp::crypto::Password::from(v));
                    }
                    Ok(out)
                }
            }

            deserializer.deserialize_map(V)
        }
    }
}
