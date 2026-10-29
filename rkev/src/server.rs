//! The HTTP side of System One, so a client written against TypeSafe's API (the
//! Agent Modeler's exported agents, for one) can be pointed at rkev instead.
//!
//! Generic over the [`SystemOne`] seam rather than tied to
//! [`LocalEngine`](crate::LocalEngine): the server needs no model, which is what
//! lets `tests/server.rs` drive it over a backend with none in it.
//!
//! A failure is never an answer. Every status but 200 ends the caller's agent
//! run, and its body says why in `{"error": {"type", "message"}}`, because the
//! exported client passes status and body on in its error message.

use std::io;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::{Error, SystemOne, SystemOneRequest, SystemOneResponse, DEFAULT_MODEL};

/// A System One server in front of one backend.
pub struct Server<B> {
    backend: B,
    api_key: Option<String>,
    models: Vec<String>,
    max_concurrent: Option<usize>,
}

impl<B> Server<B>
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    /// Serve `backend`. `Clone` because every request gets its own handle;
    /// [`LocalEngine`](crate::LocalEngine) is cheap to clone for that reason.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            api_key: None,
            models: vec![DEFAULT_MODEL.to_string()],
            max_concurrent: None,
        }
    }

    /// Require `Authorization: Bearer <key>` on every request; `None`, the
    /// default, accepts requests without the header. The exported client sends
    /// it exactly when its `RKEV_API_KEY` is set, so both of its cases fit.
    pub fn api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key;
        self
    }

    /// A name a request's `model` may ask for. The server answers to
    /// [`DEFAULT_MODEL`] (`kev-latest`) from the start, so a client that names
    /// no particular checkpoint fits; any other name is a 404. A request without
    /// a `model` is always served.
    pub fn model_name(mut self, name: impl Into<String>) -> Self {
        self.models.push(name.into());
        self
    }

    /// Answer at most `n` requests at a time and refuse the rest with 503 at
    /// once instead of queueing them. The exported client gives up after 60 s,
    /// so an answer that waited for a slot would arrive to nobody. Unlimited
    /// by default.
    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = Some(n);
        self
    }

    /// The routes, for a caller that wants to mount them in its own axum app.
    pub fn router(self) -> Router {
        let shared = Shared {
            backend: self.backend,
            authorization: self.api_key.map(|key| Arc::from(format!("Bearer {key}"))),
            models: self.models.into(),
            permits: self.max_concurrent.map(|n| Arc::new(Semaphore::new(n))),
        };
        Router::new()
            .route("/v1/systemone", post(system_one::<B>))
            .route("/health", get(health))
            .with_state(shared)
    }

    /// Answer requests on `listener` until the process ends.
    pub async fn serve(self, listener: TcpListener) -> io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

/// What every request handler gets: its handle on the backend, and the
/// `Authorization` value it must see, if any.
#[derive(Clone)]
struct Shared<B> {
    backend: B,
    authorization: Option<Arc<str>>,
    models: Arc<[String]>,
    permits: Option<Arc<Semaphore>>,
}

impl<B> Shared<B> {
    fn authorize(&self, headers: &HeaderMap) -> Result<(), Failure> {
        let Some(expected) = &self.authorization else {
            return Ok(());
        };
        let sent = headers.get(AUTHORIZATION).map(|value| value.as_bytes());
        if sent == Some(expected.as_bytes()) {
            Ok(())
        } else {
            Err(Failure {
                status: StatusCode::UNAUTHORIZED,
                kind: "unauthorized",
                message: "this server needs Authorization: Bearer <key>".to_string(),
            })
        }
    }
}

async fn system_one<B>(
    State(shared): State<Shared<B>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SystemOneResponse>, Failure>
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    shared.authorize(&headers)?;
    // Held until the answer is built, then dropped with it.
    let _permit = match &shared.permits {
        Some(permits) => Some(permits.clone().try_acquire_owned().map_err(|_| Failure {
            status: StatusCode::SERVICE_UNAVAILABLE,
            kind: "unavailable",
            message: "every slot is answering a request; try again shortly".to_string(),
        })?),
        None => None,
    };
    let backend = shared.backend;
    // Parsed here rather than by axum's `Json`, whose rejections are plain text:
    // a client should read every failure in the same shape.
    let request: SystemOneRequest = serde_json::from_slice(&body).map_err(|e| Failure {
        status: StatusCode::BAD_REQUEST,
        kind: "invalid_request",
        message: format!("the body is not a System One request: {e}"),
    })?;
    if let Some(model) = &request.model {
        if !shared.models.contains(model) {
            return Err(Failure {
                status: StatusCode::NOT_FOUND,
                kind: "model_not_found",
                message: format!(
                    "this server does not serve {model:?}; it answers to {:?}",
                    shared.models
                ),
            });
        }
    }
    Ok(Json(backend.system_one(&request).await?))
}

/// What a client reads when there is no answer: a status, and in the body the
/// error's `type` (what kind of failure) and `message` (the details).
struct Failure {
    status: StatusCode,
    kind: &'static str,
    message: String,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        let (status, kind) = match error {
            // 400 rather than kev.serve's 422: the API document says 400, and
            // the exported client stops on either.
            Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            // A request that is well formed but too long for this model: 413,
            // so a client can tell "shorten the state" from "fix the request".
            #[cfg(feature = "local")]
            Error::ContextOverflow(_) => (StatusCode::PAYLOAD_TOO_LARGE, "state_too_long"),
            #[allow(unreachable_patterns)]
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        Self {
            status,
            kind,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "type": self.kind, "message": self.message } });
        (self.status, Json(body)).into_response()
    }
}

/// Liveness for Docker and Kubernetes. A `Server` only exists around a backend
/// that is already loaded, so answering at all means ready.
async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}
