// prosody-rest-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! A way to send individual XMPP stanzas to Prosody
//! (not opening a long-living connection).

mod prosody_rest;
mod util;

pub use minidom;
pub use prose_xmpp;

pub use self::prosody_rest::{CallerCredentials, ProsodyRest};
pub use jid::BareJid;
