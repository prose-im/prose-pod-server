// prose-pod-server
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
    pub use super::{AppContext, AppState, AppStateTrait, FailState, TransitionWith as _};
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
    fn set_state<F, B>(&self, new_state: AppState<F, B>)
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
    fn new_state<F, B>(&self, new_state: AppState<F, B>) -> AppState<F, B>
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
    fn context(&self) -> Option<Arc<AppContext>> {
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
    pub fn with_backend<B2>(self, backend: B2) -> AppState<F1, B2> {
        AppState {
            app_context: self.app_context,
            frontend: self.frontend,
            backend,
        }
    }
}

pub trait AppStateTrait {
    fn state_name() -> &'static str;

    fn into_router(self) -> axum::Router;

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error>;

    fn prosody_weak(&self) -> Option<Weak<RwLock<ProsodyChildProcess>>>;
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

    use std::sync::Arc;

    use crate::{AppConfig, util::tracing_subscriber_ext::TracingReloadHandles};

    use super::macros::*;

    use self::substates::*;

    pub trait FrontendStateTrait: Into<FrontendUndergoingFactoryReset> {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles>;
    }

    // MARK: Running

    #[derive(Debug)]
    pub struct FrontendRunning<State: FrontendRunningState = Operational> {
        pub state: Arc<State>,
        pub(crate) config: Arc<AppConfig>,
        pub(crate) tracing_reload_handles: Arc<TracingReloadHandles>,
    }

    state_boilerplate!(
        FrontendRunning<any FrontendRunningState>,
        [Clone]: { state, config, tracing_reload_handles }
    );

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

        impl<'a> From<&'a Arc<anyhow::Error>> for WithMisconfiguration {
            fn from(error: &'a Arc<anyhow::Error>) -> Self {
                Self {
                    error: Arc::clone(error),
                }
            }
        }
    }

    impl<S: FrontendRunningState> FrontendStateTrait for FrontendRunning<S> {
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

    impl FrontendStateTrait for FrontendMisconfigured {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles> {
            &self.tracing_reload_handles
        }
    }

    // MARK: Factory reset

    #[derive(Debug, Clone)]
    pub struct FrontendUndergoingFactoryReset {
        pub(crate) tracing_reload_handles: Arc<TracingReloadHandles>,
    }

    state_boilerplate!(FrontendUndergoingFactoryReset);

    impl FrontendStateTrait for FrontendUndergoingFactoryReset {
        fn tracing_reload_handles(&self) -> &Arc<TracingReloadHandles> {
            &self.tracing_reload_handles
        }
    }

    // MARK: State transitions

    impl<S: FrontendStateTrait> From<(S, &Arc<anyhow::Error>)> for FrontendMisconfigured {
        fn from((state, error): (S, &Arc<anyhow::Error>)) -> Self {
            Self {
                error: Arc::clone(error),
                tracing_reload_handles: Arc::clone(state.tracing_reload_handles()),
            }
        }
    }

    impl<S: FrontendRunningState> From<(FrontendRunning<S>, &Arc<anyhow::Error>)>
        for FrontendRunning<WithMisconfiguration>
    {
        fn from((state, error): (FrontendRunning<S>, &Arc<anyhow::Error>)) -> Self {
            Self {
                state: Arc::new(WithMisconfiguration::from(error)),
                config: state.config,
                tracing_reload_handles: state.tracing_reload_handles,
            }
        }
    }

    impl<S: FrontendRunningState> From<FrontendRunning<S>> for FrontendUndergoingFactoryReset {
        fn from(state: FrontendRunning<S>) -> Self {
            Self {
                tracing_reload_handles: state.tracing_reload_handles,
            }
        }
    }

    impl From<FrontendMisconfigured> for FrontendUndergoingFactoryReset {
        fn from(state: FrontendMisconfigured) -> Self {
            Self {
                tracing_reload_handles: state.tracing_reload_handles,
            }
        }
    }

    // MARK: Boilerplate

    impl_from_pair!((FrontendRunning<Operational>, error) use left);
}

pub mod backend {
    pub mod prelude {
        pub use super::substates::*;
        pub use super::{
            BackendRunning as Running, BackendStartFailed as StartFailed,
            BackendStarting as Starting, BackendStopped as Stopped,
            BackendUndergoingFactoryReset as UndergoingFactoryReset,
        };
        #[allow(unused_imports)]
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

    use super::macros::*;

    use self::substates::*;

    // MARK: Starting

    #[derive(Debug)]
    pub struct BackendStarting<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    state_boilerplate!(
        BackendStarting<any BackendStartingState>,
        derive: Clone, Default
    );

    pub use BackendStoppedState as BackendStartingState;

    // MARK: Stopped

    #[derive(Debug)]
    pub struct BackendStopped<State: BackendStoppedState = Operational> {
        pub state: Arc<State>,
    }

    state_boilerplate!(
        BackendStopped<any BackendStoppedState>,
        derive: Clone, Default
    );

    pub trait BackendStoppedState: std::fmt::Debug {}
    impl BackendStoppedState for NotInitialized {}
    impl BackendStoppedState for Operational {}

    // MARK: Running

    #[derive(Debug)]
    pub struct BackendRunning<State: BackendRunningState = Operational> {
        pub state: Arc<State>,
    }

    state_boilerplate!(
        BackendRunning<any BackendRunningState>,
        derive: Clone, Default
    );

    pub trait BackendRunningState: std::fmt::Debug {}
    impl BackendRunningState for Operational {}

    pub mod substates {
        use crate::util::sync::AutoCancelToken;

        use super::*;

        #[derive(Debug, Default)]
        pub struct NotInitialized {}

        #[derive(Debug)]
        pub struct Operational {
            pub prosody: Arc<RwLock<ProsodyChildProcess>>,
            pub prosodyctl: Arc<RwLock<Prosodyctl>>,
            pub prosody_rest: ProsodyRest,
            pub oauth2_client: Arc<ProsodyOAuth2>,
            pub secrets_service: SecretsService,
            pub cancellation_token: AutoCancelToken,
        }
    }

    // MARK: Stopped with error

    #[derive(Debug)]
    pub struct BackendStartFailed<State: BackendStartFailedState> {
        pub state: Arc<State>,
        pub error: Arc<anyhow::Error>,
    }

    state_boilerplate!(
        BackendStartFailed<any BackendStartFailedState>,
        [Clone]: { state, error }
    );

    pub use BackendStoppedState as BackendStartFailedState;

    // MARK: Factory reset

    #[derive(Debug, Clone, Default)]
    pub struct BackendUndergoingFactoryReset {}

    state_boilerplate!(BackendUndergoingFactoryReset);

    // MARK: State transitions

    impl_trivial_transition!(
        BackendRunning<any BackendRunningState>
        => BackendStarting<any BackendStartingState>
    );
    impl_trivial_transition!(
        BackendStarting<any BackendStartingState>
        => BackendRunning<any BackendRunningState>
    );
    impl_trivial_transition!(
        BackendStartFailed<any BackendStartFailedState>
        => BackendRunning<any BackendRunningState>
    );
    impl_trivial_transition!(
        BackendUndergoingFactoryReset
        =[default]> BackendStarting<any BackendStartingState>
    );
    impl_trivial_transition!(
        BackendUndergoingFactoryReset
        =[default]> BackendStopped<NotInitialized>
    );
    impl_trivial_transition!(
        BackendStopped<NotInitialized>
        => BackendStarting<NotInitialized>
    );
    impl_trivial_transition!(
        BackendRunning<any BackendRunningState>
        =[default]> BackendUndergoingFactoryReset
    );
    impl_trivial_transition!(
        BackendStopped<any BackendStoppedState>
        =[default]> BackendUndergoingFactoryReset
    );
    impl_trivial_transition!(
        BackendStartFailed<any BackendStartFailedState>
        =[default]> BackendUndergoingFactoryReset
    );

    // MARK: Boilerplate

    impl_from_pair!((BackendRunning, error) use left);
    impl_from_pair!((BackendStopped<NotInitialized>, error) use left);
    impl_from_pair!((BackendUndergoingFactoryReset => BackendStopped<NotInitialized>, ()) use left);

    impl<S1, S2> From<(S1, &Arc<anyhow::Error>)> for BackendStartFailed<S2>
    where
        S1: Into<Arc<S2>>,
        S2: BackendStartFailedState,
    {
        #[inline(always)]
        fn from((state, error): (S1, &Arc<anyhow::Error>)) -> Self {
            Self {
                state: state.into(),
                error: Arc::clone(error),
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
        F1: Into<F2>,
        B1: Into<B2>,
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
    /// PERF(RemiBardon): This does an unnecessary `clone` if the resulting
    ///   state isn’t used (because of `new_state` instead of `set_state`).
    ///   If this becomes an issue, we could add default values to the generic
    ///   types so that the function would return `AppState<(), ()>` if the
    ///   state ends up not being used (i.e. type inference would fail).
    ///   It might be a little tricky to separate the two scenarios to avoid
    ///   the `clone` but I’m sure there is a way.
    #[must_use]
    #[inline(always)]
    fn with_transition<F2, B2>(
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
}

/// Similar to [`core::convert::From`].
pub trait TransitionFrom<T, Data> {
    fn transition_from(state: T, data: Data) -> Self;
}

/// Similar to [`core::convert::Into`].
pub trait TransitionWith<T, Data> {
    fn transition_with(self, data: Data) -> T;
}

// `TransitionFrom` implies `TransitionWith`.
impl<T, U, Data> TransitionWith<U, Data> for T
where
    U: TransitionFrom<T, Data>,
{
    #[inline]
    fn transition_with(self, data: Data) -> U {
        TransitionFrom::transition_from(self, data)
    }
}

// `AppState` transition.
impl<F, F2, FData, B, B2, BData> TransitionFrom<AppState<F, B>, (FData, BData)> for AppState<F2, B2>
where
    (F, FData): Into<F2>,
    (B, BData): Into<B2>,
    AppState<F2, B2>: AppStateTrait,
    F2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
{
    #[inline]
    fn transition_from(state: AppState<F, B>, (f_data, b_data): (FData, BData)) -> Self {
        state.with_transition::<F2, B2>(|state| AppState {
            app_context: state.app_context,
            frontend: (state.frontend, f_data).into(),
            backend: (state.backend, b_data).into(),
        })
    }
}

// MARK: Fail states

/// [`FailState`] is essentially an equivalent of `(State, Error)` which
/// provides functionality for better ergonomics. Thanks to it, we can have
/// “fluent” call sites which follow the concepts of functional programming.
pub struct FailState<F, B> {
    #[allow(dead_code)]
    pub state: AppState<F, B>,

    pub error: Arc<anyhow::Error>,
}

impl<F, B> AppState<F, B> {
    pub fn with_error(self, error: Arc<anyhow::Error>) -> FailState<F, B> {
        FailState { state: self, error }
    }
}

// `AppState` + `Arc<anyhow::Error>` to `AppState` transition.
impl<F, F2, B, B2> TransitionFrom<AppState<F, B>, Arc<anyhow::Error>> for AppState<F2, B2>
where
    AppState<F2, B2>:
        for<'a> TransitionFrom<AppState<F, B>, (&'a Arc<anyhow::Error>, &'a Arc<anyhow::Error>)>,
{
    #[inline]
    fn transition_from(state: AppState<F, B>, error: Arc<anyhow::Error>) -> Self {
        AppState::<F2, B2>::transition_from(state, (&error, &error))
    }
}

// `AppState` + `anyhow::Error` to `AppState` transition.
impl<F, B, B2> TransitionFrom<AppState<F, B>, anyhow::Error> for AppState<F, B2>
where
    AppState<F, B>: TransitionWith<Self, Arc<anyhow::Error>>,
{
    #[inline]
    fn transition_from(state: AppState<F, B>, error: anyhow::Error) -> Self {
        TransitionWith::transition_with(state, Arc::new(error))
    }
}

// `AppState` + `Arc<anyhow::Error>` to `FailState` transition.
impl<F, B> AppState<F, B> {
    #[inline]
    pub fn transition_failed<F2, B2>(self, error: anyhow::Error) -> FailState<F2, B2>
    where
        Self: for<'a> TransitionWith<
                AppState<F2, B2>,
                (&'a Arc<anyhow::Error>, &'a Arc<anyhow::Error>),
            >,
    {
        let error = Arc::new(error);
        TransitionWith::transition_with(self, (&error, &error)).with_error(error)
    }
}

/// There is a lot of repetitive things we have to do to have ergonomic
/// transitions. This is where the heavy lifting is done.
mod macros {
    macro_rules! state_boilerplate {
        (
            $state:ident<any $substate_trait:ident>
            $(, [Clone]: { $($clone_field:ident),+ })?
            $(, derive: $($trivial_impl:ident),+)?
        ) => {
            // `S { state } -> &state` (auto)
            impl<S: $substate_trait> std::ops::Deref for $state<S> {
                type Target = S;

                #[inline(always)]
                fn deref(&self) -> &Self::Target {
                    &self.state
                }
            }

            // `S { state } -> &state`
            impl<S: $substate_trait> AsRef<S> for $state<S> {
                #[inline(always)]
                fn as_ref(&self) -> &S {
                    &self.state
                }
            }

            // `S { state } -> state`
            impl<S: $substate_trait> From<$state<S>> for Arc<S> {
                #[inline(always)]
                fn from($state { state, .. }: $state<S>) -> Self {
                    state
                }
            }

            // `(S, _) -> S`
            impl<S: $substate_trait> From<($state<S>, ())> for $state<S> {
                #[inline(always)]
                fn from((state, _): ($state<S>, ())) -> Self {
                    state
                }
            }

            $(state_boilerplate!(__ [Clone]: $state<any $substate_trait> { $($clone_field)+ });)?

            $($(state_boilerplate!(__ $trivial_impl: $state<any $substate_trait>);)+)?
        };

        (
            $state:ty
            $(, [Clone]: { $($clone_field:ident),+ })?
            $(, derive: $($trivial_impl:ident),+)?
        ) => {
            // `(S, _) -> S`, `(S1, _) -> S2`
            impl From<($state, ())> for $state {
                #[inline(always)]
                fn from((state, _): ($state, ())) -> Self {
                    state.into()
                }
            }

            $(state_boilerplate!(__ [Clone]: $state { $($clone_field)+ });)?

            $($(state_boilerplate!(__ $trivial_impl: $state);)+)?
        };

        (__ Default: $state:ident<any $substate_trait:ident>) => {
            impl<S: $substate_trait + Default> Default for $state<S> {
                #[inline(always)]
                fn default() -> Self {
                    Self {
                        state: Arc::default(),
                    }
                }
            }
        };

        (__ Clone: $state:ident<any $substate_trait:ident>) => {
            state_boilerplate!(__ [Clone]: $state<any $substate_trait> { state });
        };

        (__ [Clone]: $state:ident<any $substate_trait:ident> { $($clone_field:ident)+ }) => {
            impl<S: $substate_trait> Clone for $state<S> {
                #[inline(always)]
                fn clone(&self) -> Self {
                    // NOTE: `#[derive(Clone)]` doesn’t work here,
                    //   we have to do it manually :/
                    Self {
                        $($clone_field: Arc::clone(&self.$clone_field),)+
                    }
                }
            }
        };
    }
    pub(super) use state_boilerplate;

    macro_rules! impl_trivial_transition {
        // Generic to generic.
        ($t1:ident<any $sub_t1_trait:ident> => $t2:ident<any $sub_t2_trait:ident>) => {
            impl<Substate> From<$t1<Substate>> for $t2<Substate>
            where
                Substate: $sub_t1_trait + $sub_t2_trait,
            {
                #[inline(always)]
                fn from($t1 { state, .. }: $t1<Substate>) -> Self {
                    Self { state }
                }
            }
        };

        // Generic to concrete using Default.
        (
            $t1:ident<any $sub_t1_trait:ident>
            =[default]>
            $t2:path
        ) => {
            impl<S: $sub_t1_trait> From<$t1<S>> for $t2
            where
                Self: Default,
            {
                #[inline(always)]
                fn from(_: $t1<S>) -> Self {
                    Self::default()
                }
            }
        };

        // Concrete to generic using Default.
        (
            $t1:path
            =[default]>
            $t2:ident<any $sub_t2_trait:ident>
        ) => {
            impl<S: $sub_t2_trait> From<$t1> for $t2<S>
            where
                Self: Default,
            {
                #[inline(always)]
                fn from(_: $t1) -> Self {
                    Self::default()
                }
            }
        };

        // Concrete to concrete.
        ($t1:path => $t2:path) => {
            impl From<$t1> for $t2 {
                #[inline(always)]
                fn from($t1 { state, .. }: $t1) -> Self {
                    Self { state }
                }
            }
        };

        // Concrete to concrete using Default.
        ($t1:path =[default]> $t2:path) => {
            impl From<$t1> for $t2
            where
                Self: Default,
            {
                #[inline(always)]
                fn from(_: $t1) -> Self {
                    Self::default()
                }
            }
        };
    }
    pub(super) use impl_trivial_transition;

    macro_rules! impl_from_pair {
        // Use left, discard error.
        (($left:ty, error) use left) => {
            impl<'a> From<($left, &'a Arc<anyhow::Error>)> for $left {
                #[inline(always)]
                fn from((left, _): ($left, &'a Arc<anyhow::Error>)) -> Self {
                    left
                }
            }
        };

        // Use left, discard right.
        (($left:ty, $right:ty) use left) => {
            impl From<($left, $right)> for $left {
                #[inline(always)]
                fn from((left, _): ($left, $right)) -> Self {
                    left
                }
            }
        };

        // Map left, discard right.
        (($other:ty => $left:ty, $right:ty) use left) => {
            impl From<($other, $right)> for $left
            where
                $other: Into<$left>,
            {
                #[inline(always)]
                fn from((left, _): ($other, $right)) -> Self {
                    left.into()
                }
            }
        };
    }
    pub(super) use impl_from_pair;
}
