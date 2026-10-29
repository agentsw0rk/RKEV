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

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;

use crate::{Error, SystemOne, SystemOneRequest, SystemOneResponse};

/// A System One server in front of one backend.
pub struct Server<B> {
    backend: B,
}

impl<B> Server<B>
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    /// Serve `backend`. `Clone` because every request gets its own handle;
    /// [`LocalEngine`](crate::LocalEngine) is cheap to clone for that reason.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// The routes, for a caller that wants to mount them in its own axum app.
    pub fn router(self) -> Router {
        Router::new()
            .route("/v1/systemone", post(system_one::<B>))
            .with_state(self.backend)
    }

    /// Answer requests on `listener` until the process ends.
    pub async fn serve(self, listener: TcpListener) -> io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

async fn system_one<B>(
    State(backend): State<B>,
    body: Bytes,
) -> Result<Json<SystemOneResponse>, Failure>
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    // Parsed here rather than by axum's `Json`, whose rejections are plain text:
    // a client should read every failure in the same shape.
    let request: SystemOneRequest = serde_json::from_slice(&body).map_err(|e| Failure {
        status: StatusCode::BAD_REQUEST,
        kind: "invalid_request",
        message: format!("the body is not a System One request: {e}"),
    })?;
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
