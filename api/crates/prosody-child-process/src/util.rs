// prosody-child-process-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

/// [`panic!`] in debug mode, useless in release.
macro_rules! debug_panic {
    ($($args:tt)*) => {
        if cfg!(debug_assertions) {
            panic!("[debug_only] {}", format!($($args)*));
        }
    };
}
pub(crate) use debug_panic;

/// [`panic!`] in debug mode, [`tracing::warn!`] in release.
macro_rules! debug_panic_or_log_warning {
    ($($args:tt)*) => {
        if cfg!(debug_assertions) {
            panic!("[debug_only] {}", format!($($args)*));
        } else {
            tracing::warn!($($args)*);
        }
    };
}
pub(crate) use debug_panic_or_log_warning;

pub fn unix_timestamp() -> u64 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
