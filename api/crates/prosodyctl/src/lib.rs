// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod prosody_shell;
mod prosodyctl;

pub use prosody_shell::errors::*;
pub use prosody_shell::{ProsodyResponse, ProsodyShell};
pub use prosodyctl::Prosodyctl;
#[cfg(feature = "secrecy")]
pub use secrecy;

#[cfg(not(feature = "secrecy"))]
pub type Password = str;
#[cfg(feature = "secrecy")]
pub type Password = secrecy::SecretString;
