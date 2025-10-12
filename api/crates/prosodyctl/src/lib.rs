// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod prosody_shell;
mod prosodyctl;

#[cfg(feature = "secrecy")]
pub use secrecy;

pub use self::prosody_shell::errors::*;
pub use self::prosody_shell::{ProsodyResponse, ProsodyShell};
pub use self::prosodyctl::Prosodyctl;

#[cfg(not(feature = "secrecy"))]
pub type Password = str;
#[cfg(feature = "secrecy")]
pub type Password = secrecy::SecretString;
