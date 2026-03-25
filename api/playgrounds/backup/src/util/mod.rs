// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod fs;
mod measurements;
#[cfg(feature = "provider_fs")]
mod octal;
pub mod serde;

pub use self::fs::*;
pub use self::measurements::BytesAmount;
#[cfg(feature = "provider_fs")]
pub use self::octal::Octal;

/// Casting with `as` can yield incorrect values and similar issues
/// happen with `clamp`. This function ensures no overflow happens.
#[cfg(any(feature = "provider_s3", test))]
pub fn saturating_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
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

#[inline]
pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now().unix_timestamp()
}

/// Panic in debug mode.
///
/// To use with [`Result::inspect_err`].
#[inline]
pub fn debug_panic<E: std::fmt::Debug>(error: &E) {
    if cfg!(debug_assertions) {
        panic!("{error:?}")
    }
}

/// [`panic!`] in debug mode, [`tracing::error!`] in release.
macro_rules! debug_panic_or_log_error {
    ($($args:tt)*) => {
        if cfg!(debug_assertions) {
            panic!("[debug_only] {}", format!($($args)*));
        } else {
            tracing::error!($($args)*);
        }
    };
}
pub(crate) use debug_panic_or_log_error;

macro_rules! assert_impl {
    ($ty:ty : $trait:path) => {
        #[cfg(not(coverage))]
        const _: fn() = || {
            fn assert_impl<T: $trait>() {}
            assert_impl::<$ty>();
        };
    };
}
pub(crate) use assert_impl;

/// While waiting for <https://github.com/rust-lang/rust/commit/e1424588bd6c0865d1b3425e8f67c93554733d4e>
/// to make it to a stable release.
#[cfg(feature = "provider_s3")]
pub fn get_or_try_insert<T, E>(
    opt: &mut Option<T>,
    f: impl FnOnce() -> Result<T, E>,
) -> Result<&mut T, E> {
    if opt.is_none() {
        *opt = Some(f()?);
    }

    // SAFETY: A `None` variant for `opt` would have been replaced by a `Some`
    // variant in the code above.
    Ok(opt.as_mut().unwrap())
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
