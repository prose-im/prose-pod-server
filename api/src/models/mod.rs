// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub use jid::*;
pub mod jid {
    pub use jid::{BareJid, DomainPart as JidDomain, NodePart as JidNode};
}

pub use password::*;
pub mod password {
    use secrecy::SecretString;

    #[derive(Debug, Clone)]
    #[derive(serde_with::DeserializeFromStr)]
    #[repr(transparent)]
    pub struct Password(SecretString);

    impl Password {
        // NOTE: Not just in `Default` to allow more explicit code.
        pub fn random() -> Self {
            Self(crate::util::random_strong_password())
        }
    }

    // NOTE: Allows public creation while keeping `.0` private.
    impl From<SecretString> for Password {
        fn from(secret: SecretString) -> Self {
            Self(secret)
        }
    }

    // NOTE: Allows access to `.0` (e.g. to wrap in another type) but without
    //   allowing mutability (which would bypass the minimum password length).
    impl From<Password> for SecretString {
        fn from(password: Password) -> Self {
            password.0
        }
    }

    impl Default for Password {
        fn default() -> Self {
            Self::random()
        }
    }

    impl std::ops::Deref for Password {
        type Target = SecretString;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl std::str::FromStr for Password {
        type Err = std::convert::Infallible;

        fn from_str(str: &str) -> Result<Self, Self::Err> {
            Ok(Self::from(SecretString::from(str)))
        }
    }
}

pub use auth::*;
pub mod auth {
    use secrecy::SecretString;
    use serde::Serialize;

    use crate::models::BareJid;

    #[derive(Debug, Clone)]
    #[repr(transparent)]
    pub struct AuthToken(pub SecretString);

    /// Information about who made the API call.
    #[derive(Debug, Clone)]
    #[derive(Serialize)]
    pub struct CallerInfo {
        pub jid: BareJid,
        pub primary_role: String,
    }

    // MARK: Boilerplate

    impl std::ops::Deref for AuthToken {
        type Target = SecretString;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AuthToken {
        pub fn inner(&self) -> &SecretString {
            &self.0
        }
    }

    impl<T> From<T> for AuthToken
    where
        SecretString: From<T>,
    {
        fn from(value: T) -> Self {
            Self(SecretString::from(value))
        }
    }
}

pub use color::*;
pub mod color;

pub use avatar::*;
pub mod avatar {
    use media_type_detect::{MediaType, SUPPORTED_IMAGE_MEDIA_TYPES, detect_image_media_type};
    use prosody_rest::prose_xmpp::mods::AvatarData;
    use serde::Serialize;

    // NOTE: This is the very maximum the Server API will accept. While a softer limit
    //   could be configured via the app configuration (checked when uploading), this
    //   limit ensures no [`Avatar`] value can ever exceed 10MB (to prevent abuse).
    pub(crate) const AVATAR_MAX_LENGTH_BYTES: usize = 10_000_000;

    #[derive(Debug)]
    #[serde_with::serde_as]
    #[derive(Serialize)]
    pub struct Avatar {
        /// Fixed-size slice of bytes.
        #[serde(rename = "base64")]
        #[serde_as(as = "serde_with::base64::Base64")]
        bytes: Box<[u8]>,

        /// Media type, infered from magic bytes for security reasons.
        #[serde(rename = "type")]
        media_type: MediaType,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum AvatarDecodeError {
        #[error("Avatar too large. Max length: {AVATAR_MAX_LENGTH_BYTES}B.")]
        TooLarge,
        #[error("Unsupported media type. Supported: {SUPPORTED_IMAGE_MEDIA_TYPES:?}.")]
        UnsupportedMediaType,
        #[error("Invalid Base64: {0}")]
        InvalidBase64(#[from] base64::DecodeError),
    }

    impl TryFrom<AvatarData> for Avatar {
        type Error = AvatarDecodeError;

        fn try_from(avatar_data: AvatarData) -> Result<Self, Self::Error> {
            match avatar_data {
                AvatarData::Base64(base64) => Self::try_from_base64_string(base64),
                AvatarData::Data(bytes) => Self::try_from_bytes(bytes),
            }
        }
    }

    impl Avatar {
        pub fn try_from_bytes(bytes: Box<[u8]>) -> Result<Self, AvatarDecodeError> {
            if bytes.len() > AVATAR_MAX_LENGTH_BYTES {
                return Err(AvatarDecodeError::TooLarge);
            }

            let media_type =
                detect_image_media_type(&bytes).ok_or(AvatarDecodeError::UnsupportedMediaType)?;

            Ok(Self {
                bytes,
                media_type: media_type,
            })
        }

        #[inline]
        pub fn try_from_base64(bytes: Box<[u8]>) -> Result<Self, AvatarDecodeError> {
            use base64::{Engine as _, prelude::BASE64_STANDARD};

            let bytes = BASE64_STANDARD
                .decode(bytes)
                .map_err(AvatarDecodeError::InvalidBase64)?;

            Self::try_from_bytes(bytes.into_boxed_slice())
        }

        #[inline]
        pub fn try_from_base64_string(string: String) -> Result<Self, AvatarDecodeError> {
            Self::try_from_base64(string.into_bytes().into_boxed_slice())
        }
    }

    impl Avatar {
        #[inline]
        pub fn into_bytes(self) -> Box<[u8]> {
            self.bytes
        }
    }
}
