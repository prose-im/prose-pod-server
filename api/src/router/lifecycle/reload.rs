// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::either::Either;

pub(in crate::router) async fn reload<F: frontend::State>(
    State(app_state): State<AppState<F, b::Running>>,
) -> Result<(), Error>
where
    for<'a> (F, &'a Error): Into<F>,
    AppState<F, b::Running>: AppStateTrait,
{
    match app_state.do_reload_frontend() {
        Ok(new_state) => match new_state.do_reload_backend().await {
            Ok(_new_state) => Ok(()),
            Err(FailState { error, .. }) => Err(error),
        },
        Err(FailState { error, .. }) => Err(error),
    }
}

impl AppState<f::Misconfigured, b::Stopped> {
    pub async fn do_init_config(
        self,
    ) -> Result<
        AppState<f::Running, b::Running>,
        Either<FailState<f::Misconfigured, b::Stopped>, FailState<f::Running, b::StartFailed>>,
    > {
        let app_state = self
            .do_reload_frontend::<f::Misconfigured, b::Stopped, b::Starting>()
            .map_err(Either::E1)?;

        app_state
            .set_backend_starting()
            .do_bootstrapping()
            .await
            .map_err(Either::E2)
    }
}

pub(in crate::router) async fn init_config(
    State(app_state): State<AppState<f::Misconfigured, b::Stopped>>,
) -> Result<(), Error> {
    match app_state.do_init_config().await {
        Ok(_new_state) => Ok(()),
        Err(Either::E1(FailState { error, .. })) => Err(error),
        Err(Either::E2(FailState { error, .. })) => Err(error),
    }
}

impl AppContext {
    pub fn reload(&self) {
        let router = self.router();
        tokio::task::spawn(async move {
            match Self::reload_(router).await {
                Ok(()) => {}
                Err(err) => tracing::error!("{err:?}"),
            }
        });
    }

    async fn reload_(
        router: axum_hot_swappable_router::HotSwappableRouter,
    ) -> Result<(), anyhow::Error> {
        use anyhow::Context as _;
        use tower::ServiceExt as _;

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/lifecycle/reload")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = router
            .oneshot(request)
            .await
            .unwrap_or_else(|err| match err {});

        if response.status().is_success() {
            Ok(())
        } else {
            #[derive(Debug, serde::Deserialize)]
            #[allow(dead_code)]
            pub struct Error {
                kind: Box<str>,
                code: Box<str>,
                message: Box<str>,
                description: Box<str>,
            }

            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .context("Could not read HTTP response body bytes")?;
            let error: Error = serde_json::from_slice(&bytes)
                .context("Could not decode error from HTTP response body")?;

            Err(anyhow::anyhow!("{error:?}"))
        }
    }
}
