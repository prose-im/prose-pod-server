// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

pub mod openpgp {
    use super::*;

    /// `openpgp::Fingerprint`.
    pub mod fingerprint {
        use ::openpgp::Fingerprint;

        use super::*;

        #[inline]
        pub fn deserialize<'de, D>(deserializer: D) -> Result<Fingerprint, D::Error>
        where
            D: Deserializer<'de>,
        {
            let hex = <&str>::deserialize(deserializer)?;
            Fingerprint::from_hex(hex).map_err(serde::de::Error::custom)
        }

        /// `Option<openpgp::Fingerprint>`.
        pub mod option {
            use super::*;

            #[inline]
            pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Fingerprint>, D::Error>
            where
                D: Deserializer<'de>,
            {
                Option::<&str>::deserialize(deserializer)?
                    .map(|s| Fingerprint::from_hex(s).map_err(serde::de::Error::custom))
                    .transpose()
            }
        }
    }
}
