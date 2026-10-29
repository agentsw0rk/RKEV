//! The HTTP server, spoken to the way the Agent Modeler's exported client speaks
//! to it: one request per decision node, over plain HTTP/1.1. The client is a
//! bare TcpStream rather than an HTTP library, so what is under test is what goes
//! over the wire, and the crate keeps no HTTP client among its dependencies.
#![cfg(feature = "server")]

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use rkev::server::Server;
use rkev::{
    Answer, Error, IndexMap, Result, SystemOne, SystemOneRequest, SystemOneResponse, Usage,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

/// A backend that answers from a canned response, whatever it is asked.
#[derive(Clone)]
struct Canned(SystemOneResponse);

#[allow(clippy::manual_async_fn)]
impl SystemOne for Canned {
    fn system_one(
        &self,
        _request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        async move { Ok(self.0.clone()) }
    }

    fn system_one_separate(
        &self,
        request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        self.system_one(request)
    }
}

/// A backend that fails every request with the error its function builds; a function
/// rather than an `Error` because `Error` is not `Clone`.
#[derive(Clone)]
struct Fails(fn() -> Error);

#[allow(clippy::manual_async_fn)]
impl SystemOne for Fails {
    fn system_one(
        &self,
        _request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        async move { Err((self.0)()) }
    }

    fn system_one_separate(
        &self,
        request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        self.system_one(request)
    }
}

/// A backend that holds every request until the test releases it, and says when
/// one has arrived, so a test can have a request in flight on purpose.
#[derive(Clone)]
struct Held {
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}

impl Held {
    fn new() -> Self {
        Self {
            arrived: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl SystemOne for Held {
    fn system_one(
        &self,
        _request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        async move {
            self.arrived.notify_one();
            self.release.notified().await;
            Ok(shipping())
        }
    }

    fn system_one_separate(
        &self,
        request: &SystemOneRequest,
    ) -> impl Future<Output = Result<SystemOneResponse>> + Send {
        self.system_one(request)
    }
}

fn shipping() -> SystemOneResponse {
    let mut answers = IndexMap::new();
    answers.insert(
        "decision".to_string(),
        Answer::Choice {
            choice: "shipping".to_string(),
            confidence: 0.91,
            probabilities: [
                ("returns".to_string(), 0.07),
                ("shipping".to_string(), 0.93),
            ]
            .into_iter()
            .collect(),
        },
    );
    SystemOneResponse {
        model: "kev-0.6b".to_string(),
        answers,
        usage: Usage::default(),
        latency_ms: None,
    }
}

/// The `choice` request from the API document, as the export sends it.
fn choice_request() -> Value {
    json!({
        "state": "Paket kam nie an.",
        "questions": {
            "decision": {
                "type": "choice",
                "instructions": "Welches Team?",
                "criteria": { "returns": "Rueckgabe", "shipping": "Lieferung" }
            }
        }
    })
}

async fn start<B>(server: Server<B>) -> SocketAddr
where
    B: SystemOne + Clone + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(server.serve(listener));
    address
}

/// What came back: the status code, the headers (names lower-cased) and the body.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

async fn post(address: SocketAddr, path: &str, extra_headers: &[&str], body: &str) -> Reply {
    call("POST", address, path, extra_headers, body).await
}

async fn get(address: SocketAddr, path: &str) -> Reply {
    call("GET", address, path, &[], "").await
}

/// One HTTP/1.1 request, written by hand. `Connection: close` unless the caller
/// sends its own `Connection` header; either way the reply is read by its
/// `Content-Length`, so a connection the server keeps open cannot hang the test.
async fn call(
    method: &str,
    address: SocketAddr,
    path: &str,
    extra_headers: &[&str],
    body: &str,
) -> Reply {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n",
        body.len()
    );
    if !extra_headers
        .iter()
        .any(|header| header.to_ascii_lowercase().starts_with("connection:"))
    {
        request.push_str("Connection: close\r\n");
    }
    for header in extra_headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "connection closed before the headers ended");
        raw.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8(raw[..head_end].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(key, _)| key == "content-length")
        .map(|(_, value)| value.parse().unwrap())
        .expect("a Content-Length");
    let mut body = raw[head_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "connection closed before the body ended");
        body.extend_from_slice(&chunk[..read]);
    }
    Reply {
        status,
        headers,
        body: String::from_utf8(body).unwrap(),
    }
}

#[tokio::test]
async fn a_choice_request_is_answered_with_the_backends_answer() {
    let address = start(Server::new(Canned(shipping()))).await;

    let reply = post(address, "/v1/systemone", &[], &choice_request().to_string()).await;

    assert_eq!(reply.status, 200, "body: {}", reply.body);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    let answer = &reply.json()["answers"]["decision"];
    assert_eq!(answer["type"], "choice");
    assert_eq!(answer["choice"], "shipping");
}

#[tokio::test]
async fn a_request_the_backend_rejects_is_a_400_that_says_why() {
    const REASON: &str = "questions.decision.criteria must have at least 1 option";
    let address = start(Server::new(Fails(|| Error::Invalid(REASON.to_string())))).await;

    let reply = post(address, "/v1/systemone", &[], &choice_request().to_string()).await;

    assert_eq!(reply.status, 400, "body: {}", reply.body);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    let error = &reply.json()["error"];
    assert_eq!(error["type"], "invalid_request");
    assert!(
        error["message"].as_str().unwrap().contains(REASON),
        "error: {error}"
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_400_in_the_same_error_shape() {
    let address = start(Server::new(Canned(shipping()))).await;

    let reply = post(address, "/v1/systemone", &[], r#"{"state": "cut off"#).await;

    assert_eq!(reply.status, 400, "body: {}", reply.body);
    let error = &reply.json()["error"];
    assert_eq!(error["type"], "invalid_request");
    assert!(
        !error["message"].as_str().unwrap().is_empty(),
        "error: {error}"
    );
}

#[tokio::test]
async fn with_a_key_set_a_request_without_it_is_a_401() {
    let server = Server::new(Canned(shipping())).api_key(Some("s3cret".to_string()));
    let address = start(server).await;

    let reply = post(address, "/v1/systemone", &[], &choice_request().to_string()).await;

    assert_eq!(reply.status, 401, "body: {}", reply.body);
    assert_eq!(reply.json()["error"]["type"], "unauthorized");
}

#[tokio::test]
async fn with_a_key_set_only_that_exact_key_is_let_through() {
    let server = Server::new(Canned(shipping())).api_key(Some("s3cret".to_string()));
    let address = start(server).await;
    let body = choice_request().to_string();

    let right = post(
        address,
        "/v1/systemone",
        &["Authorization: Bearer s3cret"],
        &body,
    )
    .await;
    let wrong = post(
        address,
        "/v1/systemone",
        &["Authorization: Bearer s3cre"],
        &body,
    )
    .await;

    assert_eq!(right.status, 200, "body: {}", right.body);
    assert_eq!(wrong.status, 401, "body: {}", wrong.body);
}

#[tokio::test]
async fn an_h2c_upgrade_offer_is_ignored_and_answered_over_http_1_1() {
    // What java.net.http.HttpClient adds to every request to an http:// URL.
    let address = start(Server::new(Canned(shipping()))).await;
    let java = [
        "Connection: Upgrade, HTTP2-Settings",
        "Upgrade: h2c",
        "HTTP2-Settings: AAEAAEAAAAIAAAABAAMAAABkAAQBAAAAAAUAAEAA",
    ];

    let reply = post(
        address,
        "/v1/systemone",
        &java,
        &choice_request().to_string(),
    )
    .await;

    assert_eq!(reply.status, 200, "body: {}", reply.body);
    assert_eq!(reply.json()["answers"]["decision"]["choice"], "shipping");
}

#[tokio::test]
async fn a_model_the_server_does_not_serve_is_a_404() {
    let address = start(Server::new(Canned(shipping())).model_name("kev-0.6b")).await;
    let mut request = choice_request();
    request["model"] = json!("kev-4b");

    let reply = post(address, "/v1/systemone", &[], &request.to_string()).await;

    assert_eq!(reply.status, 404, "body: {}", reply.body);
    let error = &reply.json()["error"];
    assert_eq!(error["type"], "model_not_found");
    assert!(
        error["message"].as_str().unwrap().contains("kev-4b"),
        "error: {error}"
    );
}

#[tokio::test]
async fn kev_latest_the_served_name_and_no_model_at_all_are_answered() {
    let address = start(Server::new(Canned(shipping())).model_name("kev-0.6b")).await;

    for model in [json!("kev-latest"), json!("kev-0.6b"), Value::Null] {
        let mut request = choice_request();
        if !model.is_null() {
            request["model"] = model.clone();
        }

        let reply = post(address, "/v1/systemone", &[], &request.to_string()).await;

        assert_eq!(reply.status, 200, "model {model}: {}", reply.body);
    }
}

#[cfg(feature = "local")]
#[tokio::test]
async fn a_state_too_long_for_the_model_is_a_413() {
    let address = start(Server::new(Fails(|| {
        Error::ContextOverflow("4113 tokens, the model takes 4096".to_string())
    })))
    .await;

    let reply = post(address, "/v1/systemone", &[], &choice_request().to_string()).await;

    assert_eq!(reply.status, 413, "body: {}", reply.body);
    let error = &reply.json()["error"];
    assert_eq!(error["type"], "state_too_long");
    assert!(
        error["message"].as_str().unwrap().contains("4113 tokens"),
        "error: {error}"
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn an_engine_failure_is_a_500_not_an_answer() {
    let address = start(Server::new(Fails(|| {
        Error::Engine("the forward pass failed".to_string())
    })))
    .await;

    let reply = post(address, "/v1/systemone", &[], &choice_request().to_string()).await;

    assert_eq!(reply.status, 500, "body: {}", reply.body);
    assert_eq!(reply.json()["error"]["type"], "internal");
}

#[tokio::test]
async fn a_request_past_the_concurrency_limit_is_a_503_at_once() {
    let held = Held::new();
    let address = start(Server::new(held.clone()).max_concurrent(1)).await;
    let body = choice_request().to_string();
    let first = tokio::spawn({
        let body = body.clone();
        async move { post(address, "/v1/systemone", &[], &body).await }
    });
    held.arrived.notified().await;

    let second = post(address, "/v1/systemone", &[], &body).await;

    assert_eq!(second.status, 503, "body: {}", second.body);
    assert_eq!(second.json()["error"]["type"], "unavailable");
    held.release.notify_one();
    assert_eq!(first.await.unwrap().status, 200);
}

#[tokio::test]
async fn health_answers_200_once_the_server_is_up() {
    // The server is built around a loaded backend, so being up is being ready.
    let address = start(Server::new(Canned(shipping()))).await;

    let reply = get(address, "/health").await;

    assert_eq!(reply.status, 200, "body: {}", reply.body);
    assert_eq!(reply.json()["status"], "ok");
}

#[tokio::test]
async fn models_lists_the_names_the_server_answers_to() {
    let address = start(Server::new(Canned(shipping())).model_name("kev-0.6b")).await;

    let reply = get(address, "/v1/models").await;

    assert_eq!(reply.status, 200, "body: {}", reply.body);
    assert_eq!(reply.json()["models"], json!(["kev-latest", "kev-0.6b"]));
}

#[tokio::test]
async fn with_a_key_set_the_api_needs_it_but_the_health_probe_does_not() {
    let server = Server::new(Canned(shipping())).api_key(Some("s3cret".to_string()));
    let address = start(server).await;

    assert_eq!(get(address, "/v1/models").await.status, 401);
    assert_eq!(get(address, "/health").await.status, 200);
}
