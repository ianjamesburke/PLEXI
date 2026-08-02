//! Shared minimal HTTP/1.1 JSON-RPC transport for Plexi's MCP servers.
//!
//! The host MCP server (`app::host_mcp`) speaks a single `POST /mcp` request
//! carrying a JSON-RPC body, authenticated with `Authorization: Bearer
//! <token>`. This module owns request framing, the caller-supplied credential
//! lookup, the body-size guard, and response writing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Largest request body accepted, guarding against a crafted `Content-Length`.
pub const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB

/// Outcome of reading one request.
pub enum RequestOutcome<A> {
    /// Authenticated `POST /mcp` with a parsed JSON-RPC body.
    Json { body: serde_json::Value, auth: A },
    /// A non-200 HTTP response was already written; the caller should close.
    Handled,
}

/// Read and authenticate one request on `stream`. On success returns the parsed
/// JSON-RPC body. On any protocol or auth failure, writes the appropriate HTTP
/// status to `stream` and returns [`RequestOutcome::Handled`].
pub fn read_json_rpc_request<A>(
    stream: &TcpStream,
    authenticate: impl FnOnce(&str) -> Option<A>,
) -> std::io::Result<RequestOutcome<A>> {
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
    let mut bearer = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(RequestOutcome::Handled);
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line ends headers
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            let value = trimmed
                .split_once(':')
                .map(|(_, value)| value.trim())
                .unwrap_or("");
            if value
                .get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
            {
                bearer = value.get(7..).map(|token| token.trim().to_string());
            }
        }
    }

    let Some(auth) = bearer.as_deref().and_then(authenticate) else {
        write_http_response(&mut write_stream, 401, b"{\"error\":\"unauthorized\"}")?;
        return Ok(RequestOutcome::Handled);
    };
    if !is_post_mcp || content_length == 0 {
        write_http_response(
            &mut write_stream,
            405,
            b"{\"error\":\"method not allowed\"}",
        )?;
        return Ok(RequestOutcome::Handled);
    }
    if content_length > MAX_BODY {
        write_http_response(&mut write_stream, 413, b"{\"error\":\"payload too large\"}")?;
        return Ok(RequestOutcome::Handled);
    }

    let mut buf = vec![0u8; content_length];
    reader.read_exact(&mut buf)?;
    match serde_json::from_slice(&buf) {
        Ok(body) => Ok(RequestOutcome::Json { body, auth }),
        Err(e) => {
            log::warn!("mcp_http: invalid JSON body: {e}");
            write_http_response(&mut write_stream, 400, b"{\"error\":\"invalid json\"}")?;
            Ok(RequestOutcome::Handled)
        }
    }
}

/// Write a `Connection: close` HTTP/1.1 response with a JSON body.
pub fn write_http_response(
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
