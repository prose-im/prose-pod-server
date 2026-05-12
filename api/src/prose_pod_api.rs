// prosody-http-rs
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::http::HeaderValue;
use reqwest::header::ACCEPT;
use serde::de::DeserializeOwned;
use tokio_util::io::ReaderStream;

/// Rust interface to [`mod_http_admin_api`](https://hg.prosody.im/prosody-modules/file/tip/mod_http_admin_api).
#[derive(Debug, Clone)]
pub struct ProsePodApi {
    pub http_client: Arc<reqwest::Client>,
    pub url: String,
}

// MARK: Users

impl ProsePodApi {
    pub async fn put_restore(
        &self,
        prose_token: &HeaderValue,
        data: impl tokio::io::AsyncRead + Send + 'static,
    ) -> Result<(), self::Error> {
        let response = self
            .put("/v1/backups-internal/restore")
            .body(reqwest::Body::wrap_stream(ReaderStream::new(data)))
            .header("x-prose-token", prose_token)
            .send()
            .await?;

        receive(response).await
    }
}

// MARK: - Errors

pub use self::ProsePodApiError as Error;

#[derive(Debug, thiserror::Error)]
pub enum ProsePodApiError {
    /// An unknown error happened.
    ///
    /// The request has failed at the networking layer, there was a breaking
    /// change in Prosody or we didn’t write enough tests.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<reqwest::Error> for ProsePodApiError {
    fn from(err: reqwest::Error) -> Self {
        Self::Other(anyhow::Error::new(err).context("Network error"))
    }
}

// MARK: - Helpers

#[allow(dead_code)]
impl ProsePodApi {
    fn url(&self, path: &str) -> String {
        assert!(path.starts_with('/'));
        format!("{base}{path}", base = self.url)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http_client
            .get(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http_client
            .post(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        self.http_client
            .put(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        self.http_client
            .delete(self.url(path))
            .header(ACCEPT, "application/json")
    }
}

/// NOTE: This is separated from [`ProsePodApi::get`] and similar
///   for two reasons:
///
///   1. Separate concerns.
///   2. Allow routes to do something with the response before it’s parsed.
///      It’s not something we do at the moment of writing this, but at least
///      we won’t have to rewrite everything if we need to do this.
async fn receive<Response: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<Response, self::Error> {
    use anyhow::Context as _;

    let response = response
        .error_for_status()?
        .json::<Response>()
        .await
        .context("Could not decode Prose Pod API response")?;

    Ok(response)
}
