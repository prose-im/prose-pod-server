// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod blueprints;
pub mod pgp;
pub mod s3;
pub mod test_lifecycle;

pub mod prelude {
    pub use super::blueprints::*;
    pub(crate) use super::env_required;
    pub use super::pgp::*;
    pub use super::test_lifecycle::*;
    pub use super::unique_hex;
}

pub fn unique_hex() -> String {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    format!("{:x}", ns)
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
