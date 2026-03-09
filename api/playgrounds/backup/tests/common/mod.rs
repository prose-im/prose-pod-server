// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod before_all;
pub mod blueprints;
pub mod pgp;

pub use self::before_all::*;
pub use self::blueprints::*;
pub use self::pgp::*;

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
