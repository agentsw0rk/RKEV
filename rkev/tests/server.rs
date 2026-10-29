//! The HTTP server, spoken to the way the Agent Modeler's exported client speaks
//! to it: one request per decision node, over plain HTTP/1.1. The client is a
//! bare TcpStream rather than an HTTP library, so what is under test is what goes
//! over the wire, and the crate keeps no HTTP client among its dependencies.
#![cfg(feature = "server")]

use std::future::Future;
use std::net::SocketAddr;

use rkev::server::Server;
use rkev::{Answer, IndexMap, Result, SystemOne, SystemOneRequest, SystemOneResponse, Usage};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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

/// One HTTP/1.1 request, written by hand, with `Connection: close` so the reply
/// ends where the stream does.
async fn post(address: SocketAddr, path: &str, extra_headers: &[&str], body: &str) -> Reply {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for header in extra_headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("a header block");
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_string(),
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
