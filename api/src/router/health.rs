// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! `HealthTrait` implementations for all app substates.
//!
//! Having it all in a single file allows one to
//! see all possible health errors at once.

use axum::http::StatusCode;
use axum::response::IntoResponse as _;

use crate::errors;
use crate::state::prelude::*;
use crate::util::ResponseExt as _;

pub(crate) trait HealthTrait {
    fn health(&self) -> axum::response::Response;
}

// MARK: - Backend

// MARK: Backend running

impl HealthTrait for backend::Running {
    fn health(&self) -> axum::response::Response {
        StatusCode::OK.into_response()
    }
}

// MARK: Backend stopped

impl HealthTrait for backend::Stopped {
    fn health(&self) -> axum::response::Response {
        errors::service_unavailable(
            "SERVER_STOPPED",
            "Prose Server stopped",
            "Contact an administrator to fix this.",
        )
        .into_response()
    }
}

impl HealthTrait for backend::Starting {
    fn health(&self) -> axum::response::Response {
        errors::too_early(
            "SERVER_STARTING",
            "A moment please",
            "Your Prose Server is starting.",
        )
        .into_response()
        .retry_after(1)
    }
}

impl HealthTrait for backend::StartFailed {
    fn health(&self) -> axum::response::Response {
        errors::start_failed(&self.error).into_response()
    }
}

impl HealthTrait for backend::Restarting {
    fn health(&self) -> axum::response::Response {
        errors::too_early(
            "SERVER_RESTARTING",
            "A moment please",
            "Your Prose Server is restarting.",
        )
        .into_response()
        .retry_after(1)
    }
}

impl HealthTrait for backend::RestartFailed {
    fn health(&self) -> axum::response::Response {
        errors::restart_failed(&self.error).into_response()
    }
}

// MARK: Factory reset

impl HealthTrait for backend::UndergoingFactoryReset {
    fn health(&self) -> axum::response::Response {
        errors::service_unavailable(
            "FACTORY_RESET_IN_PROGRESS",
            "Factory reset in progress",
            "Come back in a few moments to find your brand-new Prose Workspace.",
        )
        .into_response()
        // FIXME: Test if this value makes sense.
        .retry_after(15)
    }
}

// MARK: - Frontend

// MARK: Frontend running

impl HealthTrait for frontend::Running {
    fn health(&self) -> axum::response::Response {
        StatusCode::OK.into_response()
    }
}

impl HealthTrait for frontend::RunningWithMisconfiguration {
    fn health(&self) -> axum::response::Response {
        errors::bad_configuration(&self.error).into_response()
    }
}

// MARK: Frontend misconfigured

impl HealthTrait for frontend::Misconfigured {
    fn health(&self) -> axum::response::Response {
        errors::bad_configuration(&self.error).into_response()
    }
}

// MARK: Factory reset

impl HealthTrait for frontend::UndergoingFactoryReset {
    fn health(&self) -> axum::response::Response {
        errors::service_unavailable(
            "FACTORY_RESET_IN_PROGRESS",
            "Factory reset in progress",
            "Come back in a few moments to find your brand-new Prose Workspace.",
        )
        .into_response()
        // FIXME: Test if this value makes sense.
        .retry_after(15)
    }
}
