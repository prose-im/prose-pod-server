// prose-pod-server-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

// MARK: Bare JID

// TODO: Parse `BareJid`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
#[repr(transparent)]
pub struct BareJid(String);

impl BareJid {
    pub fn new(node: &JidNode, domain: &JidDomain) -> Self {
        Self(format!("{node}@{domain}"))
    }

    pub fn node(&self) -> JidNode {
        let marker_idx = self.0.find("@").expect("A bare JID should contain a ‘@’");
        JidNode(self.0[..marker_idx].to_owned())
    }

    pub fn domain(&self) -> JidDomain {
        let marker_idx = self.0.find("@").expect("A bare JID should contain a ‘@’");
        JidDomain(self.0[(marker_idx + 1)..].to_owned())
    }
}

impl std::str::FromStr for BareJid {
    type Err = &'static str;

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if !string.contains("@") {
            Err("Missing '@'.")
        } else if string.contains("/") {
            Err("Resource part not permitted.")
        } else {
            Ok(Self(string.to_owned()))
        }
    }
}

impl std::ops::Deref for BareJid {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

impl std::fmt::Display for BareJid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl From<&BareJid> for prosody_rest::prose_xmpp::BareJid {
    fn from(jid: &BareJid) -> Self {
        Self::new(&jid.to_string()).unwrap()
    }
}

// MARK: JID node

// TODO: Parse `JidNode`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
#[repr(transparent)]
pub struct JidNode(String);

impl std::str::FromStr for JidNode {
    type Err = &'static str;

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if string.contains("@") {
            Err("'@' not permitted.")
        } else if string.contains("/") {
            Err("'/' not permitted.")
        } else {
            Ok(Self(string.to_owned()))
        }
    }
}

impl std::ops::Deref for JidNode {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

impl std::fmt::Display for JidNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

// MARK: JID domain

// TODO: Parse `JidDomain`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
#[repr(transparent)]
pub struct JidDomain(String);

impl std::str::FromStr for JidDomain {
    type Err = &'static str;

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if string.contains("@") {
            Err("'@' not permitted.")
        } else if string.contains("/") {
            Err("'/' not permitted.")
        } else {
            Ok(Self(string.to_owned()))
        }
    }
}

impl std::ops::Deref for JidDomain {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

impl std::fmt::Display for JidDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}
