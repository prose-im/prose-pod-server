// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::routing::post;

use crate::responders::Error;
use crate::router::util::frontend_health;
use crate::state::prelude::*;
use crate::{AppConfig, errors};

/// **Running with misconfiguration** (after a `SIGHUP`
/// with a bad configuration).
impl AppStateTrait for AppState<f::Running<f::WithMisconfiguration>, b::Running> {
    fn state_name() -> &'static str {
        "Running with misconfiguration"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/lifecycle/reload", post(Self::frontend_reload_route))
            .fallback(frontend_health)
            .with_state(self)
    }
}

/// **Misconfigured** (after a reload).
impl AppStateTrait for AppState<f::Misconfigured, b::Running> {
    fn state_name() -> &'static str {
        "Misconfigured"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/lifecycle/reload", post(Self::frontend_reload_route))
            .fallback(frontend_health)
            .with_state(self)
    }
}

/// **Configuration needed** (after a factory reset).
impl AppStateTrait for AppState<f::Misconfigured, b::Stopped<b::NotInitialized>> {
    fn state_name() -> &'static str {
        "Configuration needed"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/lifecycle/reload", post(Self::frontend_reload_route))
            .fallback(frontend_health)
            .with_state(self)
    }
}

impl<F, B> AppState<F, B>
where
    B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
{
    /// Try reloading the frontend, but do not transition if an error occurs.
    #[inline]
    fn try_reload_frontend<B2>(
        self,
    ) -> Result<AppState<f::Running<f::Operational>, B2>, (Self, anyhow::Error)>
    where
        B2: From<B>,
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        AppState<f::Running, B2>: AppStateTrait,
    {
        match AppConfig::from_default_figment() {
            Ok(app_config) => {
                let todo =
                    "Log warn if config changed and needs a restart (e.g. server address/port).";

                let new_state = self.with_transition::<f::Running<f::Operational>, B2>(|state| {
                    state
                        .with_frontend(f::Running {
                            state: Arc::new(f::Operational {}),
                            config: Arc::new(app_config),
                        })
                        .with_backend_transition(From::from)
                });

                Ok(new_state)
            }

            Err(err) => {
                let error = anyhow::Error::new(err).context("Frontend reload failed");
                Err((self, error))
            }
        }
    }
}

impl<FrontendSubstate, B> AppState<f::Running<FrontendSubstate>, B>
where
    FrontendSubstate: FrontendRunningState,
    B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    AppState<f::Running, B>: AppStateTrait,
    AppState<f::Running<f::WithMisconfiguration>, B>: AppStateTrait,
{
    /// ```txt
    /// AppState<Running<_>, B>
    /// --------------------------------- (Reload frontend)
    /// AppState<Running<Operational>, B>
    /// ```
    #[inline]
    fn do_reload_frontend(
        self,
    ) -> Result<
        AppState<f::Running<f::Operational>, B>,
        (
            AppState<f::Running<f::WithMisconfiguration>, B>,
            Arc<anyhow::Error>,
        ),
    > {
        match self.try_reload_frontend() {
            Ok(new_state) => Ok(new_state),

            Err((app_state, error)) => {
                let error = Arc::new(error);

                // Log debug info.
                tracing::debug!("{error:?}");

                let new_state = app_state
                    .with_transition::<f::Running<f::WithMisconfiguration>, B>(|state| {
                        state.with_frontend_transition(|state| f::Running {
                            state: Arc::new(f::WithMisconfiguration {
                                error: Arc::clone(&error),
                            }),
                            config: state.config,
                        })
                    });

                Err((new_state, error))
            }
        }
    }
}

impl<FrontendSubstate> AppState<f::Running<FrontendSubstate>, b::Running>
where
    FrontendSubstate: FrontendRunningState,
{
    pub(in crate::router) async fn frontend_reload_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        match app_state.try_reload_frontend() {
            Ok(app_state) => {
                let fixme = "That shouldn’t be here";
                _ = app_state.do_reload_backend().await?;
                Ok(())
            }

            Err((_, error)) => {
                tracing::error!("{error:?}");
                Err(errors::bad_configuration(&error))
            }
        }
    }
}

impl AppState<f::Misconfigured, b::Running> {
    async fn frontend_reload_route(State(app_state): State<Self>) -> Result<(), Error> {
        match app_state.try_reload_frontend::<b::Running>() {
            Ok(app_state) => {
                let fixme = "That shouldn’t be here";
                _ = app_state.do_reload_backend().await?;
                Ok(())
            }

            // Transition state if the reload failed.
            Err((app_state, error)) => {
                let error = Arc::new(error);
                let res = errors::bad_configuration(&error);

                app_state.transition_with(|state| {
                    state.with_frontend(f::Misconfigured {
                        error: Arc::clone(&error),
                    })
                });

                Err(res)
            }
        }
    }
}

impl AppState<f::Misconfigured, b::Stopped<b::NotInitialized>> {
    async fn frontend_reload_route(State(app_state): State<Self>) -> Result<(), Error> {
        match app_state.try_reload_frontend::<b::Starting<b::NotInitialized>>() {
            Ok(app_state) => match crate::startup::bootstrap(app_state).await {
                Ok(app_state) => {
                    let fixme = "That shouldn’t be here";
                    _ = app_state.do_reload_backend().await?;
                    Ok(())
                }
                Err(err) => {
                    let todo = "Handle error";
                    panic!("{err:?}")
                }
            },

            // Transition state if the reload failed.
            Err((app_state, error)) => {
                let error = Arc::new(error);
                let res = errors::bad_configuration(&error);

                app_state.transition_with(|state| {
                    state.with_frontend(f::Misconfigured {
                        error: Arc::clone(&error),
                    })
                });

                Err(res)
            }
        }
    }
}
