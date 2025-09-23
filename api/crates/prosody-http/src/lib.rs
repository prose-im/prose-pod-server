// prosody-http-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[cfg(feature = "mod_http_oauth2")]
pub mod mod_http_oauth2;
mod util;

#[cfg(feature = "secrecy")]
pub use secrecy;

#[derive(Debug)]
pub struct ProsodyHttpConfig {
    pub url: String,
}

// impl ProsodyHttpClient {
//     pub async fn call(
//         &self,
//         make_req: impl FnOnce(&HttpClient) -> RequestBuilder,
//         accept: impl FnOnce(&ResponseData) -> bool,
//     ) -> Result<ResponseData> {
//         let client = self.http_client.clone();
//         let request = make_req(&client).build().context("Cannot build request")?;
//         trace!("Calling `{} {}`…", request.method(), request.url());

//         let request_data = match request.try_clone() {
//             Some(request_clone) => Some(RequestData::from(request_clone).await),
//             None => None,
//         };
//         let response = {
//             let response =
//                 (client.execute(request).await).context("Prosody OAuth2 API call failed")?;
//             ResponseData::from(response).await
//         };

//         if accept(&response) {
//             Ok(response)
//         } else {
//             let body = response.text();
//             trace!(
//                 "Prosody OAuth2 error: {status}: {body}",
//                 status = response.status
//             );
//             Err(match response.status {
//                 StatusCode::UNAUTHORIZED => Error::Unauthorized(body),
//                 StatusCode::FORBIDDEN => Error::Forbidden(body),
//                 StatusCode::BAD_REQUEST if body.to_lowercase().contains("invalid jid") => {
//                     Error::Unauthorized("Invalid JID".to_string())
//                 }
//                 StatusCode::BAD_REQUEST
//                     if response
//                         .text()
//                         .to_lowercase()
//                         .contains("incorrect credentials") =>
//                 {
//                     Error::Unauthorized("Incorrect credentials".to_string())
//                 }
//                 StatusCode::BAD_REQUEST if body.to_lowercase().contains("invalid_grant") => {
//                     Error::Unauthorized("Invalid token".to_string())
//                 }
//                 _ => {
//                     let err =
//                         UnexpectedHttpResponse::new(request_data, response, error_description)
//                             .await;
//                     Error::Internal(anyhow::Error::new(err).context("Unexpected API response"))
//                 }
//             })
//         }
//     }
// }

#[cfg(not(feature = "secrecy"))]
pub type Password = str;
#[cfg(feature = "secrecy")]
pub type Password = secrecy::SecretString;

pub type Error = ProsodyHttpError;
pub type Result<T> = core::result::Result<T, ProsodyHttpError>;

#[derive(Debug, thiserror::Error)]
pub enum ProsodyHttpError {
    /// Your credentials are incorrect.
    #[error("Unauthorized: {reason}")]
    Unauthorized { reason: String },

    /// You’re not allowed to do what you asked for.
    #[error("Forbidden: {reason}")]
    Forbidden { reason: String },

    /// We made a mistake somewhere.
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl ProsodyHttpError {
    pub fn unauthorized(reason: impl Into<String>) -> Self {
        Self::Unauthorized {
            reason: reason.into(),
        }
    }

    pub fn forbidden(reason: impl Into<String>) -> Self {
        Self::Forbidden {
            reason: reason.into(),
        }
    }
}

impl From<ureq::Error> for ProsodyHttpError {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::StatusCode(403) => Self::forbidden("Check Prosody logs."),
            _ => Self::Internal(anyhow::Error::new(err).context("Request error")),
        }
    }
}

impl From<ureq::http::Response<ureq::Body>> for ProsodyHttpError {
    fn from(mut response: ureq::http::Response<ureq::Body>) -> Self {
        use std::str::FromStr as _;
        use ureq::http::header::CONTENT_TYPE;

        // Fail without reading body if not in debug mode.
        if !cfg!(debug_assertions) {
            return Self::from(ureq::Error::StatusCode(response.status().as_u16()));
        }

        // Do not try to read HTML (it’s usually the default
        // Prosody HTTP response, with no useful data).
        let content_type = response.headers().get(CONTENT_TYPE);
        let is_html =
            content_type.is_some_and(|ct| ct.as_bytes().starts_with("text/html".as_bytes()));
        if is_html {
            let ureq_error = ureq::Error::StatusCode(response.status().as_u16());
            return Self::Internal(
                anyhow::Error::new(ureq_error).context("Request error (HTML body)"),
            );
        }

        // Read the response body (consumes it).
        let body = match response.body_mut().read_to_string() {
            Ok(body) => body,
            Err(_) => "<invalid body>".to_owned(),
        };

        // Try to find the error description in the JSON response body.
        let description = match serde_json::Value::from_str(&body) {
            Ok(json) => json
                .get("error_description")
                .map(serde_json::Value::as_str)
                .flatten()
                .map(ToOwned::to_owned)
                .unwrap_or(body),
            Err(_) => body,
        };

        return Self::Internal(anyhow::Error::msg(description).context("Request error"));
    }
}
