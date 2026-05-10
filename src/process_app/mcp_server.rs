//! Minimal HTTP/1.1 MCP server for the Plexi app MCP bridge (#958).
//!
//! When an app manifest declares `[app.mcp]`, the host calls `start_mcp_server`
//! which binds a TCP listener on `127.0.0.1:0` (OS-assigned port), spawns a
//! background accept thread, and returns a `McpServerHandle` carrying the port
//! and a shared call queue.
//!
//! Each frame, `ProcessApp::poll_mcp_calls` drains the queue, serialises
//! `PlexiEvent::McpToolCall` events to the app's stdin, and stores the
//! response channel in `mcp_pending`. When the app replies with
//! `HostCommand::McpToolResult`, the routing layer looks up the call_id,
//! sends the result, and the blocked HTTP handler thread unblocks and writes
//! the JSON-RPC response to the client.

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
    /// Pending tool-call requests from external MCP clients, drained each frame
    /// by `ProcessApp::poll_mcp_calls`.
    pub call_queue: Arc<Mutex<VecDeque<McpCallRequest>>>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Start the HTTP/SSE MCP server for `tools`. Returns the handle with the
/// bound port and the shared call queue.
pub fn start_mcp_server(tools: Vec<McpTool>) -> std::io::Result<McpServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let call_queue: Arc<Mutex<VecDeque<McpCallRequest>>> =
        Arc::new(Mutex::new(VecDeque::new()));
    let queue_clone = Arc::clone(&call_queue);
    let tools = Arc::new(tools);

    std::thread::Builder::new()
        .name(format!("mcp-accept-{port}"))
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tools = Arc::clone(&tools);
                        let queue = Arc::clone(&queue_clone);
                        std::thread::Builder::new()
                            .name("mcp-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &tools, &queue) {
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

    log::info!("mcp_server: started on port {port}");
    Ok(McpServerHandle { port, call_queue })
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

fn handle_connection(
    stream: std::net::TcpStream,
    tools: &[McpTool],
    queue: &Arc<Mutex<VecDeque<McpCallRequest>>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut write_stream = stream;

    // Read HTTP request line (e.g. "POST /mcp HTTP/1.1")
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    // We only handle POST /mcp
    let is_post_mcp = request_line.starts_with("POST") && request_line.contains("/mcp");

    // Read headers, find Content-Length
    let mut content_length: usize = 0;
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
            }
        }
    }

    if !is_post_mcp || content_length == 0 {
        write_http_response(&mut write_stream, 405, b"{\"error\":\"method not allowed\"}")?;
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
    let method = json.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let response_body = match method {
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

            let call_id = uuid_v4();
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

            log::info!("mcp_server: tool_call call_id={call_id} tool={tool_name}");

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
    let status_text = if status == 200 { "OK" } else { "Error" };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// Generate a simple pseudo-UUID v4 using random bytes from the OS.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Use time + thread id as entropy source — no uuid crate dependency needed.
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let tid = std::thread::current().id();
    format!("{t:08x}-{tid:?}-mcp")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}
