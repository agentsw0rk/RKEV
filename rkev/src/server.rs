//! The HTTP side of System One, so a client written against TypeSafe's API (the
//! Agent Modeler's exported agents, for one) can be pointed at rkev instead.
//!
//! Generic over the [`SystemOne`] seam rather than tied to
//! [`LocalEngine`](crate::LocalEngine): the server needs no model, which is what
//! lets `tests/server.rs` drive it over a backend with none in it.

use std::io;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use tokio::net::TcpListener;

use crate::{SystemOne, SystemOneRequest, SystemOneResponse};

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
    Json(request): Json<SystemOneRequest>,
) -> Json<SystemOneResponse>
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    // Errors get their status codes in the next step; until then a failure
    // is a dropped connection, which no client mistakes for an answer.
    Json(
        backend
            .system_one(&request)
            .await
            .expect("error mapping comes next"),
    )
}
