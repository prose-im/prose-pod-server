// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub(crate) mod prelude {
    #[allow(unused_imports)]
    pub use super::backend::{
        BackendRunningState, BackendStartFailedState, BackendStartingState, BackendStoppedState,
    };
    pub use super::backend::{prelude as backend, prelude as b};
    pub use super::frontend::FrontendRunningState;
    pub use super::frontend::{prelude as frontend, prelude as f};
    #[allow(unused_imports)]
    pub use super::{AppContext, AppState, AppStateTrait};
}

use std::sync::{Arc, Weak};

use arc_swap::ArcSwapOption;
use axum_hot_swappable_router::HotSwappableRouter;
use prosody_child_process::ProsodyChildProcess;
use tokio::sync::RwLock;

use crate::AppConfig;

/// “App state“ of the global immutable `axum::Router`.
///
/// Think of it as a place where static values are stored,
/// except they are not static by Rust terminology to support
/// having multiple HTTP APIs with different states running
/// at the same time, in an isolated manner. This is useful
/// in tests, but also to support hot-swapping `axum` routers
/// without terminating in-flight requests.
///
/// NOTE: Cannot be generic because it will be immutable.
#[derive(Clone)]
pub struct AppContext {
    router: HotSwappableRouter,
    prosody: Arc<ArcSwapOption<Weak<RwLock<ProsodyChildProcess>>>>,
}

impl Drop for AppContext {
    fn drop(&mut self) {
        if crate::SHUTTING_DOWN.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::debug!("[Drop] App context dropped")
        } else {
            panic!("[Drop] App context dropped")
        }
    }
}

impl AppContext {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            router: HotSwappableRouter::default(),
            prosody: Arc::default(),
        }
    }

    #[inline]
    pub fn set_state<F, B>(&self, new_state: AppState<F, B>)
    where
        AppState<F, B>: AppStateTrait,
        F: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        self.prosody.swap(new_state.prosody_weak().map(Arc::new));

        let router: axum::Router = crate::router::with_base_routes(
            new_state.frontend.clone(),
            new_state.backend.clone(),
            new_state.into_router(),
        );
        self.router.set(router);

        tracing::info!("State changed: {}", AppState::<F, B>::state_name())
    }

    /// Just a helper around [`set_state`](Self::set_state) which clones the
    /// value and returns it. This makes call sites more concise when using
    /// explicit generic types for clarity.
    #[inline(always)]
    pub fn new_state<F, B>(&self, new_state: AppState<F, B>) -> AppState<F, B>
    where
        AppState<F, B>: AppStateTrait,
        F: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        self.set_state(new_state.clone());
        new_state
    }

    /// WARN: Do not call this to hot-swap the router.
    ///   Instead, use [`set_state`](Self::set_state).
    #[inline(always)]
    pub fn router(&self) -> HotSwappableRouter {
        self.router.clone()
    }

    pub async fn cleanup(&self) -> Result<(), anyhow::Error> {
        match self.prosody.load().as_deref().map(Weak::upgrade) {
            Some(Some(prosody)) => {
                prosody.write().await.stop().await?;
            }
            _ => {}
        }

        Ok(())
    }
}

/// State of transient `axum::Router`s. Basically a pair of substates
/// but newtyped into a struct for better ergonomics. Also has access
/// to an [`AppContext`] to mutate the app’s router.
#[derive(Debug, Clone)]
pub struct AppState<
    FrontendState = frontend::FrontendRunning<frontend::substates::Operational>,
    BackendState = backend::BackendRunning<backend::substates::Operational>,
> {
    app_context: Weak<AppContext>,
    pub frontend: FrontendState,
    pub backend: BackendState,
}

impl<F, B> AppState<F, B> {
    #[inline]
    pub fn new(app_context: Arc<AppContext>, frontend: F, backend: B) -> Self
    where
        Self: AppStateTrait,
        F: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        app_context.new_state(Self {
            app_context: Arc::downgrade(&app_context),
            frontend,
            backend,
        })
    }

    #[inline(always)]
    pub fn context(&self) -> Option<Arc<AppContext>> {
        self.app_context.upgrade()
    }
}

impl<F1, B1> AppState<F1, B1> {
    #[must_use]
    #[inline]
    pub fn with_frontend<F2>(self, frontend: F2) -> AppState<F2, B1> {
        AppState {
            app_context: self.app_context,
            frontend,
            backend: self.backend,
        }
    }

    #[must_use]
    #[inline]
    pub fn with_frontend_transition<F2>(
        self,
        transition: impl FnOnce(F1) -> F2,
    ) -> AppState<F2, B1> {
        AppState {
            app_context: self.app_context,
            frontend: transition(self.frontend),
            backend: self.backend,
        }
    }

    #[must_use]
    #[inline]
    pub fn with_backend<B2>(self, backend: B2) -> AppState<F1, B2> {
        AppState {
            app_context: self.app_context,
            frontend: self.frontend,
            backend,
        }
    }

    #[must_use]
    #[inline]
    pub fn with_backend_transition<B2>(
        self,
        transition: impl FnOnce(B1) -> B2,
    ) -> AppState<F1, B2> {
        AppState {
            app_context: self.app_context,
            frontend: self.frontend,
            backend: transition(self.backend),
        }
    }
}

pub trait AppStateTrait {
    fn state_name() -> &'static str;

    fn into_router(self) -> axum::Router;

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error>;

    fn prosody_weak(&self) -> Option<Weak<RwLock<ProsodyChildProcess>>>;
}

macro_rules! state_boilerplate {
    ($state:ident, $substate_trait:ident) => {
        impl<S: $substate_trait> std::ops::Deref for $state<S> {
            type Target = S;

            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                &self.state
            }
        }

        impl<S: $substate_trait> AsRef<S> for $state<S> {
            #[inline(always)]
            fn as_ref(&self) -> &S {
                &self.state
            }
        }
    };
}

pub mod frontend {
    pub mod prelude {
        pub use super::FrontendRunningState as RunningState;
        pub use super::FrontendStateTrait as State;
        pub use super::substates::*;
        pub use super::{
            FrontendMisconfigured as Misconfigured, FrontendRunning as Running,
            FrontendUndergoingFactoryReset as UndergoingFactoryReset,
        };
    }

    use crate::util::tracing_subscriber_ext::TracingReloadHandles;

    use super::*;

    use self::prelude::*;

    pub trait FrontendStateTrait {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles>;
    }

    // MARK: Running

    #[derive(Debug)]
    pub struct FrontendRunning<State: FrontendRunningState = Operational> {
        pub state: Arc<State>,
        pub(crate) config: Arc<AppConfig>,
        pub(crate) tracing_reload_handles: Arc<TracingReloadHandles>,
    }

    pub trait FrontendRunningState: std::fmt::Debug {}
    impl FrontendRunningState for Operational {}
    impl FrontendRunningState for WithMisconfiguration {}

    pub mod substates {
        use std::sync::Arc;

        #[derive(Debug)]
        pub struct Operational {}

        /// [`FrontendMisconfigured`](super::FrontendMisconfigured) is used
        /// after a factory reset, when the frontend cannot even start
        /// properly because of bad configuration.
        ///
        /// `FrontendRunning<WithMisconfiguration>`, on the other end, is
        /// used to signal that the configuration on disk is incorrect, but
        /// it wasn’t applied so the frontend is still running fine. This
        /// is useful when reloading the app with `SIGHUP`, when no status
        /// or exit code can indicate something is wrong.
        #[derive(Debug)]
        pub struct WithMisconfiguration {
            pub error: Arc<anyhow::Error>,
        }
    }

    impl<S: RunningState> FrontendStateTrait for Running<S> {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles> {
            &self.tracing_reload_handles
        }
    }

    // MARK: Misconfigured

    #[derive(Debug, Clone)]
    pub struct FrontendMisconfigured {
        pub error: Arc<anyhow::Error>,
        pub(crate) tracing_reload_handles: Arc<TracingReloadHandles>,
    }

    impl FrontendStateTrait for Misconfigured {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles> {
            &self.tracing_reload_handles
        }
    }

    // MARK: Factory reset

    #[derive(Debug, Clone)]
    pub struct FrontendUndergoingFactoryReset {
        pub(crate) tracing_reload_handles: Arc<TracingReloadHandles>,
    }

    impl FrontendStateTrait for UndergoingFactoryReset {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles> {
            &self.tracing_reload_handles
        }
    }

    // MARK: Boilerplate

    state_boilerplate!(Running, RunningState);

    impl<State: RunningState> Clone for Running<State> {
        #[inline(always)]
        fn clone(&self) -> Self {
            // NOTE: `#[derive(Clone)]` doesn’t work here,
            //   we have to do it manually :/
            Self {
                state: Arc::clone(&self.state),
                config: Arc::clone(&self.config),
                tracing_reload_handles: Arc::clone(&self.tracing_reload_handles),
            }
        }
    }
}

pub mod backend {
    pub mod prelude {
        pub use super::substates::*;
        pub use super::{
            BackendRunning as Running, BackendStartFailed as StartFailed,
            BackendStarting as Starting, BackendStopped as Stopped,
            BackendUndergoingFactoryReset as UndergoingFactoryReset,
        };
        pub use super::{
            BackendRunningState as RunningState, BackendStartFailedState as StartFailedState,
            BackendStartingState as StartingState, BackendStoppedState as StoppedState,
        };
    }

    use std::sync::Arc;

    use prosody_child_process::ProsodyChildProcess;
    use prosody_http::mod_http_oauth2::ProsodyOAuth2;
    use prosody_rest::ProsodyRest;
    use prosodyctl::Prosodyctl;
    use tokio::sync::RwLock;

    use crate::secrets_service::SecretsService;

    use self::prelude::*;

    // MARK: Starting

    #[derive(Debug)]
    pub struct BackendStarting<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    pub use BackendStoppedState as BackendStartingState;

    // MARK: Stopped

    #[derive(Debug)]
    pub struct BackendStopped<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    pub trait BackendStoppedState: std::fmt::Debug {}
    impl BackendStoppedState for NotInitialized {}
    impl BackendStoppedState for Operational {}

    // MARK: Running

    #[derive(Debug)]
    pub struct BackendRunning<State: BackendRunningState = Operational> {
        pub state: Arc<State>,
    }

    pub trait BackendRunningState: std::fmt::Debug {}
    impl BackendRunningState for Operational {}

    // MARK: Stopped with error

    #[derive(Debug)]
    pub struct BackendStartFailed<State: BackendStartFailedState> {
        pub state: Arc<State>,
        pub error: Arc<anyhow::Error>,
    }

    pub use BackendStoppedState as BackendStartFailedState;

    // MARK: Factory reset

    #[derive(Debug, Clone)]
    pub struct BackendUndergoingFactoryReset {}

    pub mod substates {
        use super::*;

        #[derive(Debug)]
        pub struct NotInitialized {}

        #[derive(Debug)]
        pub struct Operational {
            pub prosody: Arc<RwLock<ProsodyChildProcess>>,
            pub prosodyctl: Arc<RwLock<Prosodyctl>>,
            pub prosody_rest: ProsodyRest,
            pub oauth2_client: Arc<ProsodyOAuth2>,
            pub secrets_service: SecretsService,
        }
    }

    // MARK: State transitions

    impl<Substate> From<Running<Substate>> for Starting<Substate>
    where
        Substate: RunningState + StartingState,
    {
        #[inline(always)]
        fn from(Running { state, .. }: Running<Substate>) -> Self {
            Self { state }
        }
    }

    impl<Substate> From<Starting<Substate>> for Running<Substate>
    where
        Substate: StartingState + RunningState,
    {
        #[inline(always)]
        fn from(Starting { state, .. }: Starting<Substate>) -> Self {
            Self { state }
        }
    }

    impl<Substate> From<Starting<Substate>> for Arc<Substate>
    where
        Substate: StartingState,
    {
        #[inline(always)]
        fn from(Starting { state, .. }: Starting<Substate>) -> Self {
            state
        }
    }

    impl<Substate> From<StartFailed<Substate>> for Running<Substate>
    where
        Substate: StartFailedState + RunningState,
    {
        #[inline(always)]
        fn from(StartFailed { state, .. }: StartFailed<Substate>) -> Self {
            Self { state }
        }
    }

    impl<Substate> From<StartFailed<Substate>> for Arc<Substate>
    where
        Substate: StartFailedState,
    {
        #[inline(always)]
        fn from(StartFailed { state, .. }: StartFailed<Substate>) -> Self {
            state
        }
    }

    impl From<Stopped<NotInitialized>> for Starting<NotInitialized> {
        #[inline(always)]
        fn from(Stopped { state, .. }: Stopped<NotInitialized>) -> Self {
            Self { state }
        }
    }

    impl From<UndergoingFactoryReset> for Starting<NotInitialized> {
        #[inline(always)]
        fn from(_: UndergoingFactoryReset) -> Self {
            Self {
                state: Arc::new(NotInitialized {}),
            }
        }
    }

    // MARK: Boilerplate

    state_boilerplate!(Running, RunningState);
    state_boilerplate!(Stopped, StoppedState);

    impl<State: BackendStoppedState> Clone for BackendStarting<State> {
        #[inline(always)]
        fn clone(&self) -> Self {
            // NOTE: `#[derive(Clone)]` doesn’t work here,
            //   we have to do it manually :/
            Self {
                state: Arc::clone(&self.state),
            }
        }
    }

    impl<State: BackendRunningState> Clone for BackendRunning<State> {
        #[inline(always)]
        fn clone(&self) -> Self {
            // NOTE: `#[derive(Clone)]` doesn’t work here,
            //   we have to do it manually :/
            Self {
                state: Arc::clone(&self.state),
            }
        }
    }

    impl<State: BackendStartFailedState> Clone for BackendStartFailed<State> {
        #[inline(always)]
        fn clone(&self) -> Self {
            // NOTE: `#[derive(Clone)]` doesn’t work here,
            //   we have to do it manually :/
            Self {
                state: Arc::clone(&self.state),
                error: Arc::clone(&self.error),
            }
        }
    }

    impl<State: BackendStoppedState> Clone for BackendStopped<State> {
        #[inline(always)]
        fn clone(&self) -> Self {
            // NOTE: `#[derive(Clone)]` doesn’t work here,
            //   we have to do it manually :/
            Self {
                state: Arc::clone(&self.state),
            }
        }
    }
}

// MARK: App state transitions

const STATIC_APP_CONTEXT: &'static str = "Static router should hold app context forever";

impl<F1, B1> AppState<F1, B1> {
    #[must_use]
    #[inline]
    pub fn with_auto_transition<F2, B2>(self) -> AppState<F2, B2>
    where
        F2: From<F1>,
        B2: From<B1>,
        AppState<F2, B2>: AppStateTrait,
        F2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        let app_context = self.context().expect(STATIC_APP_CONTEXT);
        let new_state = AppState {
            app_context: Arc::downgrade(&app_context),
            frontend: self.frontend.into(),
            backend: self.backend.into(),
        };
        app_context.new_state(new_state)
    }

    /// Just like [`AppContext::new_state`], this is a helper for
    /// [`AppContext::set_state`]. It takes care of cloning
    /// `app_context` which is required by the borrow checker.
    ///
    /// If you don’t need the result, avoid a clone by using
    /// [`transition_with`](Self::transition_with) instead.
    #[must_use]
    #[inline(always)]
    pub fn with_transition<F2, B2>(
        self,
        transition: impl FnOnce(Self) -> AppState<F2, B2>,
    ) -> AppState<F2, B2>
    where
        AppState<F2, B2>: AppStateTrait,
        F2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        let app_context = self.context().expect(STATIC_APP_CONTEXT);
        app_context.new_state(transition(self))
    }

    /// This is a helper for [`AppContext::new_state`]. Just like
    /// [`with_transition`](Self::with_transition), it takes care of cloning
    /// `app_context` which is required by the borrow checker.
    #[inline(always)]
    pub fn transition_with<F2, B2>(self, transition: impl FnOnce(Self) -> AppState<F2, B2>)
    where
        AppState<F2, B2>: AppStateTrait,
        F2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        let app_context = self.context().expect(STATIC_APP_CONTEXT);
        app_context.set_state(transition(self))
    }
}
