// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod serde;

/// Casting with `as` can yield incorrect values and similar issues
/// happen with `clamp`. This function ensures no overflow happens.
pub fn saturating_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

/// Renames a directory (moves it to a new location), temporarily backing up
/// the existing directory and restoring it on failure (if applicable).
pub fn safe_replace(
    src: impl AsRef<std::path::Path>,
    dst: &std::path::Path,
) -> std::io::Result<()> {
    use std::fs;

    let backup = dst.with_extension("bak");

    if fs::exists(dst)? {
        fs::rename(dst, &backup)?;
    }

    if let Some(parent) = dst.parent() {
        if !fs::exists(parent).unwrap_or(false) {
            fs::create_dir_all(parent)?;
        }
    }

    match fs::rename(src, &dst) {
        res @ Ok(_) => {
            if backup.exists() {
                // Remove backup on success.
                fs::remove_dir_all(backup)?;
            }

            res
        }
        res @ Err(_) => {
            // Restore backup on failure.
            if backup.exists() {
                fs::rename(backup, &dst)?;
            }

            res
        }
    }
}

pub trait SystemTimeExt {
    fn unix_timestamp(&self) -> u64;
}

impl SystemTimeExt for std::time::SystemTime {
    #[inline]
    fn unix_timestamp(&self) -> u64 {
        use std::time::{Duration, UNIX_EPOCH};

        self.duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}

/// Panic in debug mode.
///
/// To use with [`Result::inspect_err`].
#[inline(always)]
pub fn debug_panic<E: std::fmt::Debug>(error: &E) {
    if cfg!(debug_assertions) {
        panic!("{error:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saturating_i64_to_u64() {
        // Casting with `as` can yield incorrect values:
        assert_eq!(i64::MIN, -9223372036854775808);
        assert_eq!(i64::MIN as u64, 9223372036854775808);

        assert_eq!(u64::MIN, 0);
        assert_eq!(saturating_i64_to_u64(i64::MIN), 0);
        assert_eq!(i64::MAX, 9223372036854775807);
        assert_eq!(saturating_i64_to_u64(i64::MAX), 9223372036854775807);
    }
}
