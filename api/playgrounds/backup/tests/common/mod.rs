// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod before_all;
pub mod blueprints;
pub mod pgp;
pub mod stores;

pub use self::before_all::*;
pub use self::blueprints::*;
pub use self::pgp::*;
pub use self::stores::*;

pub fn unique_hex() -> String {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    format!("{:x}", ns)
}
