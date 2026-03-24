// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod blueprints;
pub mod fs;
pub mod lifecycle;
pub mod pgp;
pub mod print;
#[cfg(feature = "provider_s3")]
pub mod s3;

pub mod prelude {
    pub use super::blueprints::*;
    pub(crate) use super::env_required;
    pub use super::fs::*;
    pub use super::lifecycle::*;
    pub use super::pgp::*;
    pub use super::unique_hex;
}

// NOTE: Implementation cannot be time-based, even with nanosecond precision,
//   as tests are ran concurrently and such conflicts happen (very often).
//   When it does, one test cleaning up its temporary directory causes another
//   to fail. We don’t want that.
pub fn unique_hex() -> Result<String, std::io::Error> {
    use std::io::Read as _;

    let mut urandom = std::fs::File::open("/dev/urandom")?;
    let mut buf = [0u8; 4]; // 4 bytes = 8 hex chars
    urandom.read_exact(&mut buf)?;

    let hex = buf.iter().map(|b| format!("{:02x}", b)).collect();

    Ok(hex)
}

macro_rules! env_required {
    ($name:literal) => {
        std::env::var($name).expect(concat!(
            "Environment variable `",
            $name,
            "` should be defined"
        ))
    };
}
pub(crate) use env_required;

macro_rules! log_error {
    () => {
        #[inline]
        |error| tracing::error!("{:#}", error)
    };
}
pub(crate) use log_error;
