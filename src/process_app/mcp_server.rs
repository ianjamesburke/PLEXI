//! Minimal HTTP/1.1 MCP server for the Plexi app MCP bridge (#958).
//!
//! When an app manifest declares `[app.mcp]`, the host calls `start_mcp_server`
//! which binds a TCP listener on `127.0.0.1:0` (OS-assigned port), spawns a
//! background accept thread, and returns a `McpServerHandle` carrying the port,
//! a per-app auth token, and a shared call queue.
//!
//! Each frame, `ProcessApp::poll_mcp_calls` drains the queue, serialises
//! `PlexiEvent::McpToolCall` events to the app's stdin, and stores the
//! response channel in `mcp_pending`. When the app replies with
//! `HostCommand::McpToolResult`, the routing layer looks up the call_id,
//! sends the result, and the blocked HTTP handler thread unblocks and writes
//! the JSON-RPC response to the client.
//!
//! ## Authentication
//! Every request must carry `Authorization: Bearer <token>` where `<token>` is
//! injected as `PLEXI_MCP_TOKEN` into the app environment at launch. Requests
//! without a matching token are rejected with HTTP 401 before any body is read.

use crate::app_registry::McpTool;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A pending MCP tool-call request queued for delivery to the app.
pub struct McpCallRequest {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    /// The HTTP handler thread blocks on this channel waiting for the app's reply.
    pub response_tx: std::sync::mpsc::SyncSender<McpToolResponse>,
}

/// The app's reply to a `PlexiEvent::McpToolCall`.
pub struct McpToolResponse {
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Handle returned by `start_mcp_server`. Kept alive in `ProcessApp`.
pub struct McpServerHandle {
    /// The OS-assigned port. Injected as `PLEXI_MCP_PORT` into the app process.
    pub port: u16,
    /// Per-app bearer token. Injected as `PLEXI_MCP_TOKEN` into the app process.
    /// Every incoming request must present this token in `Authorization: Bearer <token>`.
    pub token: String,
    /// Pending tool-call requests from external MCP clients, drained each frame
    /// by `ProcessApp::poll_mcp_calls`.
    pub call_queue: Arc<Mutex<VecDeque<McpCallRequest>>>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Start the HTTP/SSE MCP server for `tools`. Returns the handle with the
/// bound port, per-app auth token, and the shared call queue.
pub fn start_mcp_server(tools: Vec<McpTool>) -> std::io::Result<McpServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let call_queue: Arc<Mutex<VecDeque<McpCallRequest>>> =
        Arc::new(Mutex::new(VecDeque::new()));
    let queue_clone = Arc::clone(&call_queue);
    let tools = Arc::new(tools);
    let token_arc: Arc<String> = Arc::new(token.clone());

    std::thread::Builder::new()
        .name(format!("mcp-accept-{port}"))
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tools = Arc::clone(&tools);
                        let queue = Arc::clone(&queue_clone);
                        let token = Arc::clone(&token_arc);
                        std::thread::Builder::new()
                            .name("mcp-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &tools, &queue, &token) {
                                    log::warn!("mcp_server: connection error: {e}");
                                }
                            })
                            .ok();
                    }
                    Err(e) => {
                        log::warn!("mcp_server: accept error: {e}");
                        break;
                    }
                }
            }
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    log::info!("mcp_server: started on 127.0.0.1:{port}");
    Ok(McpServerHandle { port, token, call_queue })
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

fn handle_connection(
    stream: std::net::TcpStream,
    tools: &[McpTool],
    queue: &Arc<Mutex<VecDeque<McpCallRequest>>>,
    token: &str,
) -> std::io::Result<()> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut write_stream = stream;

    // Read HTTP request line (e.g. "POST /mcp HTTP/1.1")
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    // Only handle POST /mcp (exact path match)
    let mut req_parts = request_line.split_whitespace();
    let method = req_parts.next().unwrap_or("").to_string();
    let path = req_parts.next().unwrap_or("").to_string();
    let is_post_mcp = method == "POST" && path == "/mcp";

    // Read headers — collect Content-Length and Authorization.
    let mut content_length: usize = 0;
    let mut auth_ok = false;
    let expected_auth = format!("bearer {}", token.to_lowercase());
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header)? {
            0 => return Ok(()),
            _ => {
                let trimmed = header.trim_end_matches(|c| c == '\r' || c == '\n');
                if trimmed.is_empty() {
                    break; // blank line = end of headers
                }
                let lower = trimmed.to_lowercase();
                if let Some(rest) = lower.strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                }
                if let Some(rest) = lower.strip_prefix("authorization:") {
                    auth_ok = rest.trim() == expected_auth;
                }
            }
        }
    }

    // Reject unauthenticated requests before reading the body.
    if !auth_ok {
        log::warn!("mcp_server: auth rejected peer={peer} method={method} path={path}");
        write_http_response(&mut write_stream, 401, b"{\"error\":\"unauthorized\"}")?;
        return Ok(());
    }

    if !is_post_mcp || content_length == 0 {
        write_http_response(&mut write_stream, 405, b"{\"error\":\"method not allowed\"}")?;
        return Ok(());
    }

    // Cap body size to prevent OOM from a crafted Content-Length header.
    const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB
    if content_length > MAX_BODY {
        write_http_response(&mut write_stream, 413, b"{\"error\":\"payload too large\"}")?;
        return Ok(());
    }
    // Read body (content_length bytes)
    let mut buf = vec![0u8; content_length];
    {
        use std::io::Read;
        reader.read_exact(&mut buf)?;
    }
    let body = buf;

    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("mcp_server: invalid JSON body: {e}");
            write_http_response(&mut write_stream, 400, b"{\"error\":\"invalid json\"}")?;
            return Ok(());
        }
    };

    let id = json.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method_name = json.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let response_body = match method_name {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "plexi-mcp-bridge", "version": "1.0.0" }
            }
        }),

        "notifications/initialized" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": serde_json::Value::Null,
            "result": serde_json::Value::Null
        }),

        "tools/list" => {
            let tool_list: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                })
                .collect();
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tool_list }
            })
        }

        "tools/call" => {
            let params = json.get("params").cloned().unwrap_or_default();
            let tool_name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));

            let call_id = generate_call_id();
            let (response_tx, response_rx) =
                std::sync::mpsc::sync_channel::<McpToolResponse>(1);

            {
                let mut q = queue.lock().unwrap();
                q.push_back(McpCallRequest {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    arguments,
                    response_tx,
                });
            }

            log::info!("mcp_server: tool_call call_id={call_id} tool={tool_name} peer={peer}");

            match response_rx.recv_timeout(Duration::from_secs(30)) {
                Ok(resp) => {
                    if let Some(result) = resp.result {
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": result
                        })
                    } else {
                        let msg = resp.error.unwrap_or_else(|| "tool call failed".to_string());
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32603, "message": msg }
                        })
                    }
                }
                Err(_) => {
                    log::warn!("mcp_server: tool call {call_id} timed out after 30s");
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": "tool call timed out" }
                    })
                }
            }
        }

        other => {
            log::warn!("mcp_server: unknown method '{other}'");
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {other}") }
            })
        }
    };

    let body_bytes = serde_json::to_vec(&response_body)
        .unwrap_or_else(|_| b"{}".to_vec());
    write_http_response(&mut write_stream, 200, &body_bytes)?;
    Ok(())
}

fn write_http_response(
    stream: &mut impl Write,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn generate_call_id() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn post_mcp(port: u16, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        let auth_header = match token {
            Some(t) => format!("Authorization: Bearer {t}\r\n"),
            None => String::new(),
        };
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\n{auth_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();

        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let response_str = String::from_utf8_lossy(&response);
        let status: u16 = response_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body_start = response_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(response.len());
        (status, response[body_start..].to_vec())
    }

    #[test]
    fn test_no_auth_returns_401() {
        let handle = start_mcp_server(vec![]).unwrap();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let (status, _) = post_mcp(handle.port, None, body);
        assert_eq!(status, 401);
    }

    #[test]
    fn test_wrong_token_returns_401() {
        let handle = start_mcp_server(vec![]).unwrap();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let (status, _) = post_mcp(handle.port, Some("wrong-token"), body);
        assert_eq!(status, 401);
    }

    #[test]
    fn test_correct_token_tools_list_returns_200() {
        let handle = start_mcp_server(vec![]).unwrap();
        let token = handle.token.clone();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let (status, resp_body) = post_mcp(handle.port, Some(&token), body);
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["result"]["tools"], serde_json::json!([]));
    }

    #[test]
    fn test_correct_token_initialize_returns_200() {
        let handle = start_mcp_server(vec![]).unwrap();
        let token = handle.token.clone();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
        let (status, resp_body) = post_mcp(handle.port, Some(&token), body);
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn test_oversized_body_returns_413() {
        let handle = start_mcp_server(vec![]).unwrap();
        let token = handle.token.clone();
        // Send a legitimate-looking request but with a content-length exceeding MAX_BODY.
        // We only send the headers — the server checks content_length before reading.
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", handle.port)).unwrap();
        let huge_len = 10 * 1024 * 1024 + 1; // 1 byte over the 10 MB cap
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {huge_len}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let response_str = String::from_utf8_lossy(&response);
        let status: u16 = response_str
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        assert_eq!(status, 413);
    }

    #[test]
    fn test_each_server_has_unique_token() {
        let h1 = start_mcp_server(vec![]).unwrap();
        let h2 = start_mcp_server(vec![]).unwrap();
        assert_ne!(h1.token, h2.token);
    }
}
