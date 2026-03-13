// prose-pod-server
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::fmt::Display;

// MARK: - Bytes

/// Bytes.
///
/// See <https://en.wikipedia.org/wiki/Byte#Multiple-byte_units>.
///
/// NOTE: Named `BytesAmount` not to conflict with `bytes::Bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[derive(serde_with::SerializeDisplay, serde_with::DeserializeFromStr)]
pub enum BytesAmount {
    Bytes(u64),
    KiloBytes(u64),
    KibiBytes(u64),
    MegaBytes(u64),
    MebiBytes(u64),
    GigaBytes(u64),
    GibiBytes(u64),
}

impl BytesAmount {
    pub fn as_bytes(&self) -> u64 {
        match self {
            Self::Bytes(n) => *n,
            Self::KiloBytes(n) => n * 1000,
            Self::KibiBytes(n) => n * 1024,
            Self::MegaBytes(n) => n * 1000 * 1000,
            Self::MebiBytes(n) => n * 1024 * 1024,
            Self::GigaBytes(n) => n * 1000 * 1000 * 1000,
            Self::GibiBytes(n) => n * 1024 * 1024 * 1024,
        }
    }
}

impl std::str::FromStr for BytesAmount {
    type Err = ParseMeasurementError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (n, unit) = util::split_leading_digits(s);
        let n = u64::from_str(n).map_err(ParseMeasurementError::InvalidQuantity)?;
        match unit {
            "B" => Ok(Self::Bytes(n)),
            "kB" => Ok(Self::KiloBytes(n)),
            "KiB" => Ok(Self::KibiBytes(n)),
            "MB" => Ok(Self::MegaBytes(n)),
            "MiB" => Ok(Self::MebiBytes(n)),
            "GB" => Ok(Self::GigaBytes(n)),
            "GiB" => Ok(Self::GibiBytes(n)),
            _ => Err(ParseMeasurementError::InvalidUnit),
        }
    }
}

impl Display for BytesAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes(n) => write!(f, "{n}B"),
            Self::KiloBytes(n) => write!(f, "{n}kB"),
            Self::KibiBytes(n) => write!(f, "{n}KiB"),
            Self::MegaBytes(n) => write!(f, "{n}MB"),
            Self::MebiBytes(n) => write!(f, "{n}MiB"),
            Self::GigaBytes(n) => write!(f, "{n}GB"),
            Self::GibiBytes(n) => write!(f, "{n}GiB"),
        }
    }
}

// MARK: - Errors

#[derive(Debug, thiserror::Error)]
pub enum ParseMeasurementError {
    #[error("Invalid quantity")]
    InvalidQuantity(#[source] std::num::ParseIntError),

    #[error("Invalid unit")]
    InvalidUnit,
}

// MARK: - Helpers

mod util {
    /// `"10MB"` -> `("10", "MB")`
    pub(super) fn split_leading_digits(s: &str) -> (&str, &str) {
        let split_index = (s.char_indices())
            .find(|&(_, c)| !c.is_ascii_digit())
            .map(|(i, _)| i)
            .unwrap_or(s.len()); // If all are digits

        s.split_at(split_index)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_split_leading_digits() {
            assert_eq!(split_leading_digits("10MB"), ("10", "MB"));
            assert_eq!(split_leading_digits("10"), ("10", ""));
            assert_eq!(split_leading_digits("MB"), ("", "MB"));
            assert_eq!(split_leading_digits(""), ("", ""));
            assert_eq!(split_leading_digits("10Mb/s"), ("10", "Mb/s"));
            assert_eq!(split_leading_digits("10MB123"), ("10", "MB123"));
        }
    }
}
