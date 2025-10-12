// prosody-rest-rs
//
// Copyright: 2024-2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use async_trait::async_trait;
use jid::{BareJid, FullJid, ResourcePart};
use minidom::Element;
use prose_xmpp::{
    RequestError,
    client::ConnectorProvider,
    connector::{
        Connection as ConnectionTrait, ConnectionError, ConnectionEvent, ConnectionEventHandler,
        Connector as ConnectorTrait,
    },
    mods,
};
use secrecy::SecretString;

/// Rust interface to [`mod_rest`](https://hg.prosody.im/prosody-modules/file/tip/mod_rest).
#[derive(Clone)]
pub struct ProsodyRest {
    api_url: String,
    id_provider: Arc<dyn prose_xmpp::IDProvider>,
}

impl ProsodyRest {
    #[must_use]
    #[inline]
    pub fn new(api_url: String) -> Self {
        Self {
            api_url,
            id_provider: Arc::new(prose_xmpp::UUIDProvider::new()),
        }
    }

    #[must_use]
    #[inline]
    pub fn standard(server_http_url: String) -> Self {
        Self::new(format!("{}/rest", server_http_url))
    }
}

// MARK: - Helper functions

#[derive(Debug, Clone)]
pub struct CallerCredentials {
    pub bare_jid: BareJid,
    pub auth_token: SecretString,
}

#[derive(Debug, thiserror::Error)]
pub enum ProsodyRestError {
    #[error("{0}")]
    ConnectionError(#[from] ConnectionError),
    #[error("{0}")]
    RequestError(#[from] RequestError),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl ProsodyRest {
    #[must_use]
    pub async fn get_vcard(
        &self,
        jid: &BareJid,
        caller: &CallerCredentials,
    ) -> Result<Option<prose_xmpp::stanza::VCard4>, ProsodyRestError> {
        let xmpp_client = self.xmpp_client(caller).await?;
        let profile = xmpp_client.get_mod::<mods::Profile>();

        let vcard_opt = profile.load_vcard4(jid.to_owned()).await?;

        Ok(vcard_opt)
    }

    #[must_use]
    pub async fn set_own_vcard(
        &self,
        vcard: prose_xmpp::stanza::VCard4,
        caller: &CallerCredentials,
    ) -> Result<(), ProsodyRestError> {
        let xmpp_client = self.xmpp_client(caller).await?;
        let profile = xmpp_client.get_mod::<mods::Profile>();

        profile.publish_vcard4(vcard, None).await?;

        Ok(())
    }

    #[must_use]
    pub async fn get_avatar(
        &self,
        jid: &BareJid,
        caller: &CallerCredentials,
    ) -> Result<Option<mods::AvatarData>, ProsodyRestError> {
        use std::str::FromStr as _;
        use xmpp_parsers::hashes::Sha1HexAttribute;

        let xmpp_client = self.xmpp_client(caller).await?;
        let profile = xmpp_client.get_mod::<mods::Profile>();

        let Some(avatar_metadata) = profile.load_latest_avatar_metadata(jid).await? else {
            return Ok(None);
        };

        let avatar_data_opt = profile
            .load_avatar_image(
                jid.to_owned(),
                &Sha1HexAttribute::from_str(avatar_metadata.id.as_ref()).unwrap(),
            )
            .await?;

        Ok(avatar_data_opt)
    }

    /// Inspired by <https://github.com/prose-im/prose-core-client/blob/adae6b5a5ec6ca550c2402a75b57e17ef50583f9/crates/prose-core-client/src/app/services/account_service.rs#L116-L157>.
    #[must_use]
    pub async fn set_own_avatar(
        &self,
        avatar: Box<[u8]>,
        caller: &CallerCredentials,
    ) -> Result<(), ProsodyRestError> {
        use anyhow::{Context as _, anyhow};
        use media_type_detect::{SUPPORTED_IMAGE_MEDIA_TYPES, detect_image_media_type};
        use prose_xmpp::mods::profile::AvatarData;
        use prose_xmpp::stanza::avatar::ImageId;

        let media_type = detect_image_media_type(&avatar).ok_or(anyhow!(
            "Unsupported media type. Supported: {SUPPORTED_IMAGE_MEDIA_TYPES:?}."
        ))?;
        let dimensions = media_metadata::parse_dimensions(&avatar, &media_type)
            .context("Could not find dimensions in the avatar data")?;

        let xmpp_client = self.xmpp_client(caller).await?;
        let profile = xmpp_client.get_mod::<mods::Profile>();

        let image_data_len = avatar.len();
        let image_data = AvatarData::Data(avatar);
        let checksum: ImageId = image_data
            .generate_sha1_checksum()
            .context("Could not generate avatar checksum")?;

        profile
            .set_avatar_image(&checksum, image_data.base64())
            .await
            .context("Could not upload avatar")?;

        profile
            .set_avatar_metadata(
                image_data_len,
                &checksum,
                media_type,
                dimensions.width,
                dimensions.height,
            )
            .await
            .context("Could not upload avatar metadata")?;

        Ok(())
    }
}

impl ProsodyRest {
    #[must_use]
    async fn xmpp_client(
        &self,
        caller: &CallerCredentials,
    ) -> Result<prose_xmpp::Client, ConnectionError> {
        let xmpp_client = prose_xmpp::Client::builder()
            .set_connector_provider(self.connector_provider())
            .build();

        xmpp_client
            .connect(
                &caller
                    .bare_jid
                    .with_resource(&ResourcePart::new(&self.id_provider.new_id()).unwrap()),
                caller.auth_token.clone(),
            )
            .await?;

        Ok(xmpp_client)
    }
}

// MARK: - Plumbing

// MARK: Connector

struct ProsodyRestConnector {
    api_url: String,
}

impl ProsodyRest {
    fn connector_provider(&self) -> ConnectorProvider {
        let api_url = self.api_url.clone();
        Box::new(move || {
            Box::new(ProsodyRestConnector {
                api_url: api_url.clone(),
            })
        })
    }
}

#[async_trait]
impl ConnectorTrait for ProsodyRestConnector {
    async fn connect(
        &self,
        jid: &FullJid,
        password: SecretString,
        event_handler: ConnectionEventHandler,
    ) -> Result<Box<dyn ConnectionTrait>, ConnectionError> {
        let connection = ProsodyRestConnection {
            rest_api_url: self.api_url.clone(),
            jid: jid.clone(),
            prosody_token: password,
            event_handler: Arc::new(event_handler),
        };
        Ok(Box::new(connection))
    }
}

// MARK: Connection

#[derive(Clone)]
struct ProsodyRestConnection {
    rest_api_url: String,
    jid: FullJid,
    prosody_token: SecretString,
    event_handler: Arc<ConnectionEventHandler>,
}

impl ProsodyRestConnection {
    fn receive_stanza(&self, stanza: impl Into<Element>) {
        let ref event_handler = self.event_handler;
        let conn = self.clone();

        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async move {
                (event_handler)(Box::new(conn), ConnectionEvent::Stanza(stanza.into())).await
            })
        })
    }
}

impl ConnectionTrait for ProsodyRestConnection {
    fn send_stanza(&self, stanza: Element) -> anyhow::Result<()> {
        use crate::util::RequestBuilderExt as _;
        use anyhow::bail;
        use ureq::ResponseExt as _;

        let request_body = String::from(&stanza);
        // trace!(
        //     "Sending stanza as <{}>: {stanza:#?}…\nSerialized `request_body`: {request_body}",
        //     self.jid.read().clone().unwrap(),
        // );
        let mut response = ureq::post(&self.rest_api_url)
            .content_type("application/xmpp+xml")
            .bearer_auth(&self.prosody_token)
            .send(request_body)?;

        if !response.status().is_success() {
            let uri = response.get_uri().to_owned();
            let status = response.status();
            let body = (response.body_mut())
                .read_to_string()
                .unwrap_or_else(|err| format!("<error: {err}>"));
            bail!("POST {uri} failed ({status}): {body}");
        }

        let response_body = response.body_mut().read_to_string()?;

        if response_body.as_str() == "not-authorized" {
            bail!("Not authorized (auth token possibly expired).");
        }

        let xml = format!(r#"<wrapper xmlns="jabber:client">{response_body}</wrapper>"#);
        let wrapper = xml.parse::<Element>()?;
        let Some(stanza) = wrapper.get_child("iq", "jabber:client") else {
            bail!("Prosody response is not an `iq` stanza (`{response_body}`).");
        };
        self.receive_stanza(stanza.to_owned());

        Ok(())
    }

    fn disconnect(&self) {}
}

// MARK: - Boilerplate

impl std::fmt::Debug for ProsodyRestConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProsodyRestConnection")
            .field("rest_api_url", &self.rest_api_url)
            .field("jid", &self.jid)
            .finish()
    }
}

impl std::fmt::Debug for ProsodyRest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProsodyRest")
            .field("api_url", &self.api_url)
            .finish()
    }
}
