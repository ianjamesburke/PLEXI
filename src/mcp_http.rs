//! Shared minimal HTTP/1.1 JSON-RPC transport for Plexi's MCP servers.
//!
//! Both the per-app MCP bridge (`process_app::mcp_server`) and the host-level
//! event MCP server (`app::host_mcp`) speak the same wire protocol: a single
//! `POST /mcp` request carrying a JSON-RPC body, authenticated with
//! `Authorization: Bearer <token>`. This module owns the request framing, auth
//! check, body-size guard, and response writer so neither server duplicates it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Largest request body accepted, guarding against a crafted `Content-Length`.
pub const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB

/// Outcome of reading one request.
pub enum RequestOutcome {
    /// Authenticated `POST /mcp` with a parsed JSON-RPC body.
    Json(serde_json::Value),
    /// A non-200 HTTP response was already written; the caller should close.
    Handled,
}

/// Read and authenticate one request on `stream`. On success returns the parsed
/// JSON-RPC body. On any protocol or auth failure, writes the appropriate HTTP
/// status to `stream` and returns [`RequestOutcome::Handled`].
pub fn read_json_rpc_request(stream: &TcpStream, token: &str) -> std::io::Result<RequestOutcome> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut write_stream = stream.try_clone()?;

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(RequestOutcome::Handled);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let is_post_mcp = method == "POST" && path == "/mcp";

    let mut content_length: usize = 0;
    let mut auth_ok = false;
    let expected_auth = format!("bearer {}", token.to_lowercase());
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(RequestOutcome::Handled);
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line ends headers
        }
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
        if let Some(rest) = lower.strip_prefix("authorization:") {
            auth_ok = rest.trim() == expected_auth;
        }
    }

    if !auth_ok {
        write_http_response(&mut write_stream, 401, b"{\"error\":\"unauthorized\"}")?;
        return Ok(RequestOutcome::Handled);
    }
    if !is_post_mcp || content_length == 0 {
        write_http_response(&mut write_stream, 405, b"{\"error\":\"method not allowed\"}")?;
        return Ok(RequestOutcome::Handled);
    }
    if content_length > MAX_BODY {
        write_http_response(&mut write_stream, 413, b"{\"error\":\"payload too large\"}")?;
        return Ok(RequestOutcome::Handled);
    }

    let mut buf = vec![0u8; content_length];
    reader.read_exact(&mut buf)?;
    match serde_json::from_slice(&buf) {
        Ok(v) => Ok(RequestOutcome::Json(v)),
        Err(e) => {
            log::warn!("mcp_http: invalid JSON body: {e}");
            write_http_response(&mut write_stream, 400, b"{\"error\":\"invalid json\"}")?;
            Ok(RequestOutcome::Handled)
        }
    }
}

/// Write a `Connection: close` HTTP/1.1 response with a JSON body.
pub fn write_http_response(stream: &mut impl Write, status: u16, body: &[u8]) -> std::io::Result<()> {
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
