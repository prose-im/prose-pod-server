// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub(crate) mod prelude {
    pub use super::{
        AppContext, AppState, AppStateTrait,
        backend::{
            BackendRunningState, BackendStartFailedState, BackendStartingState, BackendStoppedState,
        },
        backend::{prelude as backend, prelude as b},
        frontend::FrontendRunningState,
        frontend::{prelude as frontend, prelude as f},
    };
}

use std::sync::Arc;

use axum_hot_swappable_router::HotSwappableRouter;
use prosody_http::mod_http_oauth2::ProsodyOAuth2Client;
use prosody_rest::ProsodyRest;
use tokio::sync::RwLock;

use crate::{AppConfig, secrets_service::SecretsService};

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
#[derive(Debug, Clone)]
pub struct AppContext {
    router: HotSwappableRouter,
}

impl AppContext {
    #[inline]
    fn new() -> Self {
        Self {
            router: HotSwappableRouter::default(),
        }
    }

    #[inline]
    pub fn set_state<F, B>(&self, new_state: AppState<F, B>)
    where
        AppState<F, B>: AppStateTrait,
        F: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
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
    #[inline]
    pub fn router(&self) -> HotSwappableRouter {
        self.router.clone()
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
    app_context: AppContext,
    pub frontend: FrontendState,
    pub backend: BackendState,
}

impl<F, B> AppState<F, B> {
    #[inline]
    pub fn new(frontend: F, backend: B) -> Self
    where
        Self: AppStateTrait,
        F: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    {
        let app_context = AppContext::new();
        app_context.new_state(Self {
            app_context: app_context.clone(),
            frontend,
            backend,
        })
    }

    #[inline(always)]
    pub fn context(&self) -> &AppContext {
        &self.app_context
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

// State transitions are enforced by `rustc`.
//
// Startup:
//   ! (no state)
// Bad config at startup:
//   ! (crash)
// Prosody crash at startup:
//   ! (crash)
// Startup (started):
//   AppState<ApiOperational, BackendOperational>
//
// During POST /backend/reload:
//   AppState<ApiOperational, BackendOperational>
// Backend reload succeeded:
//   AppState<ApiOperational, BackendOperational>
//   (+ HTTP success)
// Backend reload failed:
//   AppState<ApiOperational, BackendOperational>
//   (+ HTTP client error)
//
// During POST /backend/restart:
//   AppState<ApiOperational, BackendStopped>
// Backend restart succeeded:
//   AppState<ApiOperational, BackendOperational>
//   (+ HTTP success)
// Backend restart failed:
//   AppState<ApiOperational, RestartFailed>
//   (+ HTTP client error)
//
// During POST /lifecycle/reload:
//   AppState<ApiOperational, BackendOperational>
// Self reload succeeded:
//   AppState<ApiOperational, BackendOperational>
//   (+ HTTP success)
// Self reload failed:
//   AppState<ApiMisconfigured, BackendOperational>
//   (+ HTTP client error)
//
// During POST /lifecycle/factory-reset:
//   AppState<ApiUndergoingFactoryReset, BackendStopped>
// Factory reset succeeded:
//   AppState<ApiMisconfigured, BackendInitialized>
//     -> BackendInitialized necessary to have a handle on Prosody for future reloads
//   / AppState<ApiMisconfigured, BackendInitialized> if config in env is invalid
//   / AppState<ApiOperational, BackendOperational> if server domain in env
//   (+ HTTP success)
// Factory reset failed (unexpected error):
//   AppState<ApiUndergoingFactoryReset, BackendStopped>
//   (+ HTTP client error)
//
// SIGHUP:
//   (Prosody keeps running as if nothing happened, but throws
//   an error every time prosodyctl is invoked (status included).
//   Running shells don’t stop though, and c2s seems to still work.)
//   -> Report SERVICE_UNAVAILABLE, but keep Prosody running.

// impl ToRouter for AppState<ApiOperational, BackendOperational> {
//   // All routes
// }
// impl ToRouter for AppState<ApiOperational, BackendStopped> {
//   // POST /backend/restart
// }
// impl ToRouter for AppState<ApiMisconfigured, BackendInitialized> {
//   // POST /lifecycle/reload
// }
// impl ToRouter for AppState<ApiMisconfigured, BackendOperational> {
//   // POST /lifecycle/reload
// }
// impl ToRouter for AppState<ApiUndergoingFactoryReset, BackendStopped> {
//   // Nothing? -> Fix by hand
//   // We could have POST /backend/restart, but that’d be a bit risky.
// }

// 1. Normal case
// 2. &Backend restart
//    1. Stop
//    2. Start
//    3. -> OAuth 2.0 client conserved, service account secrets conserved
// 3. Normal case
// 4. Backend restart (failing)
//    1. Stop
//    2. Cannot start
//    3. -> OAuth 2.0 client conserved, service account secrets conserved
//    4. *Backend restart
// 5. Normal case

pub trait AppStateTrait {
    fn state_name() -> &'static str;

    fn into_router(self) -> axum::Router;
}

pub mod frontend {
    pub mod prelude {
        pub use super::FrontendRunningState as RunningState;
        pub use super::substates::*;
        pub use super::{
            FrontendMisconfigured as Misconfigured, FrontendRunning as Running,
            FrontendUndergoingFactoryReset as UndergoingFactoryReset,
        };
    }

    use std::ops::Deref;

    use super::*;

    use self::prelude::*;

    // MARK: Running

    #[derive(Debug)]
    pub struct FrontendRunning<State: FrontendRunningState = Operational> {
        pub state: Arc<State>,
        pub(crate) config: Arc<AppConfig>,
    }

    impl<State: FrontendRunningState> Clone for FrontendRunning<State> {
        fn clone(&self) -> Self {
            let todo = "Derive?";
            Self {
                state: self.state.clone(),
                config: self.config.clone(),
            }
        }
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

    // MARK: Misconfigured

    #[derive(Debug, Clone)]
    pub struct FrontendMisconfigured {
        pub error: Arc<anyhow::Error>,
    }

    // MARK: Factory reset

    #[derive(Debug, Clone)]
    pub struct FrontendUndergoingFactoryReset {}

    // MARK: State transitions

    impl<Substate> From<Running<Substate>> for UndergoingFactoryReset
    where
        Substate: RunningState,
    {
        #[inline(always)]
        fn from(_: Running<Substate>) -> Self {
            Self {}
        }
    }

    // MARK: Boilerplate

    impl<S: RunningState> std::ops::Deref for Running<S> {
        type Target = S;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }

    impl<S: RunningState, Substate> AsRef<Substate> for Running<S>
    where
        S: AsRef<Substate>,
    {
        #[inline(always)]
        fn as_ref(&self) -> &Substate {
            self.state.deref().as_ref()
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

    use std::ops::Deref;

    use prosody_child_process::ProsodyChildProcess;
    use prosodyctl::Prosodyctl;

    use super::*;

    use self::prelude::*;

    // MARK: Starting

    #[derive(Debug)]
    pub struct BackendStarting<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    impl<State: BackendStoppedState> Clone for BackendStarting<State> {
        fn clone(&self) -> Self {
            let todo = "Derive?";
            Self {
                state: self.state.clone(),
            }
        }
    }

    pub use self::BackendStoppedState as BackendStartingState;

    // MARK: Stopped

    #[derive(Debug)]
    pub struct BackendStopped<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    impl<State: BackendStoppedState> Clone for BackendStopped<State> {
        fn clone(&self) -> Self {
            let todo = "Derive?";
            Self {
                state: self.state.clone(),
            }
        }
    }

    pub trait BackendStoppedState: std::fmt::Debug {}
    impl BackendStoppedState for NotInitialized {}
    impl BackendStoppedState for Operational {}

    // MARK: Running

    #[derive(Debug)]
    pub struct BackendRunning<State: BackendRunningState = Operational> {
        pub state: Arc<State>,
    }

    impl<State: BackendRunningState> Clone for BackendRunning<State> {
        fn clone(&self) -> Self {
            let todo = "Derive?";
            Self {
                state: self.state.clone(),
            }
        }
    }

    pub trait BackendRunningState: std::fmt::Debug {}
    impl BackendRunningState for Operational {}

    // MARK: Stopped with error

    #[derive(Debug)]
    pub struct BackendStartFailed<State: BackendStartFailedState = Operational> {
        pub state: Arc<State>,
        pub error: Arc<anyhow::Error>,
    }

    impl<State: BackendStartFailedState> Clone for BackendStartFailed<State> {
        fn clone(&self) -> Self {
            let todo = "Derive?";
            Self {
                state: self.state.clone(),
                error: self.error.clone(),
            }
        }
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
            pub oauth2_client: Arc<ProsodyOAuth2Client>,
            pub secrets_service: SecretsService,
        }

        // MARK: Substate transitions

        impl From<Operational> for BackendUndergoingFactoryReset {
            #[inline(always)]
            fn from(_: Operational) -> Self {
                Self {}
            }
        }
    }

    // MARK: State transitions

    impl<Substate> From<Running<Substate>> for Stopped<Substate>
    where
        Substate: RunningState + StoppedState,
    {
        #[inline(always)]
        fn from(value: Running<Substate>) -> Self {
            Self { state: value.state }
        }
    }

    impl<Substate> From<Running<Substate>> for Starting<Substate>
    where
        Substate: RunningState + StartingState,
    {
        #[inline(always)]
        fn from(value: Running<Substate>) -> Self {
            Self { state: value.state }
        }
    }

    impl<Substate> From<Starting<Substate>> for Running<Substate>
    where
        Substate: StartingState + RunningState,
    {
        #[inline(always)]
        fn from(value: Starting<Substate>) -> Self {
            Self { state: value.state }
        }
    }

    impl<Substate> From<Stopped<Substate>> for Running<Substate>
    where
        Substate: StoppedState + RunningState,
    {
        #[inline(always)]
        fn from(value: Stopped<Substate>) -> Self {
            Self { state: value.state }
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

    impl<Substate> From<Running<Substate>> for UndergoingFactoryReset
    where
        Substate: RunningState,
    {
        #[inline(always)]
        fn from(_: Running<Substate>) -> Self {
            Self {}
        }
    }

    impl<Substate> From<Stopped<Substate>> for UndergoingFactoryReset
    where
        Substate: StoppedState,
    {
        #[inline(always)]
        fn from(_: Stopped<Substate>) -> Self {
            Self {}
        }
    }

    impl From<UndergoingFactoryReset> for Stopped<NotInitialized> {
        #[inline(always)]
        fn from(_: UndergoingFactoryReset) -> Self {
            Self {
                state: Arc::new(NotInitialized {}),
            }
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

    impl From<Stopped<NotInitialized>> for Starting<NotInitialized> {
        #[inline(always)]
        fn from(Stopped { state, .. }: Stopped<NotInitialized>) -> Self {
            Self { state }
        }
    }

    impl From<Starting<Operational>> for Arc<Operational> {
        #[inline(always)]
        fn from(value: Starting<Operational>) -> Self {
            value.state
        }
    }

    impl From<Stopped<Operational>> for Arc<Operational> {
        #[inline(always)]
        fn from(value: Stopped<Operational>) -> Self {
            value.state
        }
    }

    impl From<StartFailed<Operational>> for Arc<Operational> {
        #[inline(always)]
        fn from(value: StartFailed<Operational>) -> Self {
            value.state
        }
    }

    // MARK: Boilerplate

    impl<S: RunningState> Deref for Running<S> {
        type Target = S;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }

    impl<S: RunningState> AsRef<S> for Running<S> {
        #[inline(always)]
        fn as_ref(&self) -> &S {
            &self.state
        }
    }

    impl<S: StoppedState> Deref for Stopped<S> {
        type Target = S;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }
}

// MARK: App state transitions

impl<F1, B1> AppState<F1, B1> {
    // #[must_use]
    // #[inline]
    // pub fn transition<F2, B2>(self)
    // where
    //     F2: From<F1>,
    //     B2: From<B1>,
    //     AppState<F2, B2>: AppStateTrait,
    //     F2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    //     B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    // {
    //     let app_context = self.app_context.clone();
    //     let new_state = AppState {
    //         app_context: self.app_context,
    //         frontend: self.frontend.into(),
    //         backend: self.backend.into(),
    //     };
    //     app_context.set_state(new_state)
    // }

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
        let app_context = self.app_context.clone();
        let new_state = AppState {
            app_context: self.app_context,
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
        let app_context = self.app_context.clone();
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
        let app_context = self.app_context.clone();
        app_context.set_state(transition(self))
    }
}
