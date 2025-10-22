// prosody-child-process-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

/// [`panic!`] in debug mode, useless in release.
#[inline(always)]
pub fn debug_panic(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if cfg!(debug_assertions) {
        panic!("[debug_only] {msg}");
    }
}

/// [`panic!`] in debug mode, [`tracing::warn!`] in release.
#[inline(always)]
pub fn debug_panic_or_log_warning(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if cfg!(debug_assertions) {
        panic!("[debug_only] {msg}");
    } else {
        tracing::warn!(msg);
    }
}

pub fn unix_timestamp() -> u64 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
