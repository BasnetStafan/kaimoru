//! The upstream JSON-RPC client: one stateless POST per
//! tool call to the streamable-HTTP endpoint, carrying
//! the call through verbatim — arguments in, the
//! upstream's result (structured content included) or its
//! error out. The upstream is the store's own MCP server,
//! which is sessionless, so no handshake or session
//! header is needed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, ErrorCode, JsonObject};
use serde_json::{Value, json};

/// How long one proxied call may take in total before the
/// adapter answers with an error: a hung upstream must
/// never hang the client's session with it.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

/// How long connecting to the upstream may take.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The answer when the upstream cannot be reached at all.
const UPSTREAM_UNREACHABLE_MESSAGE: &str = "the upstream MCP endpoint could not be reached";

/// The answer when the upstream is reachable but too
/// slow: a different failure from unreachability, and
/// the client deserves to hear which one it was.
const UPSTREAM_TIMEOUT_MESSAGE: &str = "the upstream MCP endpoint did not answer in time";

/// Picks the transport-failure answer for a reqwest
/// error: the timeout message for a timeout, the
/// unreachable message for everything else.
fn transport_error_message(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        UPSTREAM_TIMEOUT_MESSAGE
    } else {
        UPSTREAM_UNREACHABLE_MESSAGE
    }
}

/// The answer when the upstream answers something that is
/// not a JSON-RPC response.
const UPSTREAM_PROTOCOL_MESSAGE: &str = "the upstream MCP endpoint returned an unreadable answer";

/// The answer when the upstream refuses at the HTTP layer
/// with no JSON-RPC error body (a host guard, a proxy):
/// the status is the only fact worth reporting.
fn upstream_status_message(status: reqwest::StatusCode) -> String {
    format!("the upstream MCP endpoint answered HTTP {status}")
}

/// Why reading an upstream response failed: either the
/// upstream sent a JSON-RPC error (carried through
/// verbatim) or its answer could not be read as one.
enum ResponseError {
    /// The upstream's own JSON-RPC error.
    Rpc(McpError),
    /// The body was not a JSON-RPC response at all.
    Unreadable,
}

/// A JSON-RPC client for one upstream MCP endpoint. The
/// API key, when configured, is forwarded as the bearer
/// token on every call and never logged. No `Debug`: the
/// key must not reach a log line through a format call.
#[derive(Clone)]
pub struct UpstreamClient {
    /// The HTTP client (pooled, rustls).
    client: reqwest::Client,
    /// The upstream endpoint URL.
    endpoint: String,
    /// The API key to forward, when configured.
    api_key: Option<String>,
    /// JSON-RPC request ids, one per call.
    next_id: Arc<AtomicU64>,
}

impl UpstreamClient {
    /// Builds a client for one endpoint.
    ///
    /// # Panics
    /// Panics when the HTTP client cannot be built. The
    /// client is rustls with the bundled WebPKI roots —
    /// nothing is read from the environment or the disk —
    /// so a build failure means a broken binary, not a
    /// configuration problem.
    pub fn new(endpoint: String, api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
            .timeout(UPSTREAM_TIMEOUT)
            .build()
            .expect("the HTTP client builds from bundled rustls roots");
        Self {
            client,
            endpoint,
            api_key,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Proxies one tool call to the upstream and returns
    /// its answer: the result exactly as the upstream
    /// shaped it, or the upstream's own error.
    ///
    /// # Errors
    /// Returns the upstream's JSON-RPC error verbatim when
    /// it sends one; a generic internal error when the
    /// upstream cannot be reached, times out, or answers
    /// something unreadable (the detail is logged, never
    /// sent to the client).
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments.unwrap_or_default() },
        });
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&body);
        if let Some(api_key) = &self.api_key {
            // The key is the upstream's own credential for
            // this adapter; forwarding it is the point of
            // the setting, and it is never logged.
            request = request.bearer_auth(api_key);
        }
        let response = request.send().await.map_err(|error| {
            tracing::warn!("MCP upstream call failed: {error}");
            McpError::internal_error(transport_error_message(&error), None)
        })?;
        let status = response.status();
        let event_stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        let text = response.text().await.map_err(|error| {
            tracing::warn!("MCP upstream response read failed: {error}");
            McpError::internal_error(transport_error_message(&error), None)
        })?;
        match read_result(&text, event_stream, id) {
            Ok(result) => Ok(result),
            Err(ResponseError::Rpc(error)) => Err(error),
            Err(ResponseError::Unreadable) => {
                if status.is_success() {
                    Err(McpError::internal_error(UPSTREAM_PROTOCOL_MESSAGE, None))
                } else {
                    Err(McpError::internal_error(
                        upstream_status_message(status),
                        None,
                    ))
                }
            }
        }
    }
}

/// Reads a tool call's answer out of an upstream response
/// body: a single JSON document, or the matching message
/// of an event stream. A JSON-RPC error becomes
/// [`ResponseError::Rpc`] with its code, message and data
/// preserved.
fn read_result(text: &str, event_stream: bool, id: u64) -> Result<CallToolResult, ResponseError> {
    let envelope = if event_stream {
        event_stream_envelope(text, id)
    } else {
        serde_json::from_str::<Value>(text).ok()
    };
    let Some(envelope) = envelope else {
        return Err(ResponseError::Unreadable);
    };
    if let Some(error) = envelope.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok());
        let message = error.get("message").and_then(Value::as_str);
        return match (code, message) {
            (Some(code), Some(message)) => Err(ResponseError::Rpc(McpError::new(
                ErrorCode(code),
                message.to_string(),
                error.get("data").cloned(),
            ))),
            _ => Err(ResponseError::Unreadable),
        };
    }
    match envelope.get("result") {
        Some(result) => serde_json::from_value::<CallToolResult>(result.clone())
            .map_err(|_| ResponseError::Unreadable),
        None => Err(ResponseError::Unreadable),
    }
}

/// Picks the JSON-RPC message for request `id` out of an
/// event-stream body (`data:` lines). Falls back to the
/// last parseable message: a sessionless upstream sends
/// exactly one response per request, so the fallback is
/// for servers that answer without echoing the id.
fn event_stream_envelope(text: &str, id: u64) -> Option<Value> {
    let mut fallback = None;
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            return Some(value);
        }
        fallback = Some(value);
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn reads_a_json_result_with_its_structured_content() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{}"}],"structuredContent":{"total":2},"isError":false}}"#;
        let result = match read_result(body, false, 1) {
            Ok(result) => result,
            Err(_) => panic!("a result envelope reads"),
        };
        let structured = result.structured_content.expect("structured content kept");
        assert_eq!(structured["total"], json!(2));
    }

    #[test]
    fn carries_an_error_through_with_its_code_and_message() {
        let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"a valid API key with the cart:write scope is required"}}"#;
        match read_result(body, false, 1) {
            Err(ResponseError::Rpc(error)) => {
                assert_eq!(error.code.0, -32001);
                assert_eq!(
                    error.message,
                    "a valid API key with the cart:write scope is required"
                );
            }
            _ => panic!("an error envelope maps to the RPC error"),
        }
    }

    #[test]
    fn picks_the_matching_message_out_of_an_event_stream() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"content\":[]}}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"hit\"}]}}\n\n";
        let result = match read_result(body, true, 9) {
            Ok(result) => result,
            Err(_) => panic!("the matching stream message reads"),
        };
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn rejects_a_body_that_is_not_json_rpc() {
        assert!(matches!(
            read_result("<html>not found</html>", false, 1),
            Err(ResponseError::Unreadable)
        ));
        assert!(matches!(
            read_result("{\"jsonrpc\":\"2.0\",\"id\":1}", false, 1),
            Err(ResponseError::Unreadable)
        ));
    }

    /// Serves one canned HTTP answer on a loopback port
    /// and returns the port and the captured request.
    fn stub_upstream(answer: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("stub binds");
        let port = listener.local_addr().expect("stub address").port();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("stub accepts");
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).is_err() {
                    break;
                }
                head.push(byte[0]);
            }
            let head_text = String::from_utf8_lossy(&head).to_string();
            let length: usize = head_text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).expect("stub reads the body");
            let _ = sender.send(format!("{head_text}\n{}", String::from_utf8_lossy(&body)));
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                answer.len()
            );
            stream.write_all(response.as_bytes()).expect("stub answers");
        });
        (port, receiver)
    }

    #[tokio::test]
    async fn proxies_a_call_and_forwards_the_api_key() {
        let (port, receiver) = stub_upstream(
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[],"structuredContent":{"total":1},"isError":false}}"#,
        );
        let client = UpstreamClient::new(
            format!("http://127.0.0.1:{port}/mcp"),
            Some("kmcp_test_secret".to_string()),
        );
        let result = client
            .call_tool("search_products", None)
            .await
            .expect("the stub answers");
        let structured = result.structured_content.expect("structured content kept");
        assert_eq!(structured["total"], json!(1));
        let request = receiver.recv().expect("the stub saw the request");
        assert!(
            request.contains("authorization: Bearer kmcp_test_secret"),
            "the key is forwarded: {request}"
        );
        assert!(
            request.contains("\"name\":\"search_products\""),
            "the call is forwarded: {request}"
        );
    }

    #[tokio::test]
    async fn a_dead_upstream_answers_a_clean_error() {
        // A port that was just released refuses connections
        // immediately, so the call must fail fast and clean.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("probe binds");
            listener.local_addr().expect("probe address").port()
        };
        let client = UpstreamClient::new(format!("http://127.0.0.1:{port}/mcp"), None);
        let error = client
            .call_tool("search_products", None)
            .await
            .expect_err("a dead upstream errors");
        assert_eq!(error.message, UPSTREAM_UNREACHABLE_MESSAGE);
    }
}
