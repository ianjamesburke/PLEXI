//! Shared minimal HTTP/1.1 JSON-RPC transport for Plexi's MCP servers.
//!
//! The host MCP server (`app::host_mcp`) speaks a single `POST /mcp` request
//! carrying a JSON-RPC body, authenticated with `Authorization: Bearer
//! <token>`. This module owns request framing, socket read/write timeouts,
//! the caller-supplied credential lookup, the body-size guard, and response
//! writing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Largest request body accepted, guarding against a crafted `Content-Length`.
pub const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB

/// Default socket read and write timeout applied to each MCP HTTP connection.
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Short drain budget when closing a connection after an early error response so
/// unread client body bytes cannot trigger a kernel TCP `RST`.
const ERROR_DRAIN_TIMEOUT: Duration = Duration::from_millis(50);
const ERROR_DRAIN_MAX_BYTES: u64 = 64 * 1024;

/// Outcome of reading one request.
pub enum RequestOutcome<A> {
    /// Authenticated `POST /mcp` with a parsed JSON-RPC body.
    Json { body: serde_json::Value, auth: A },
    /// A non-200 HTTP response was already written; the caller should close.
    Handled,
}

fn is_timeout_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Apply read and write timeouts to an accepted MCP HTTP socket.
pub fn configure_stream_timeouts(stream: &TcpStream, timeout: Duration) -> std::io::Result<()> {
    let bounded = if timeout.is_zero() {
        Duration::from_millis(1)
    } else {
        timeout
    };
    stream.set_read_timeout(Some(bounded))?;
    stream.set_write_timeout(Some(bounded))?;
    Ok(())
}

fn respond_and_close<A>(
    stream: &TcpStream,
    reader: &mut BufReader<&TcpStream>,
    status: u16,
    reason: &str,
    body: &[u8],
) -> std::io::Result<RequestOutcome<A>> {
    log::info!("mcp_http: responding status={status} reason={reason}");
    let mut write_stream = stream;
    let write_result = write_http_response(&mut write_stream, status, body);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(ERROR_DRAIN_TIMEOUT));
    let _ = std::io::copy(
        &mut reader.take(ERROR_DRAIN_MAX_BYTES),
        &mut std::io::sink(),
    );
    write_result?;
    Ok(RequestOutcome::Handled)
}

/// Read and authenticate one request on `stream`. On success returns the parsed
/// JSON-RPC body. On any protocol, timeout, or auth failure, writes the
/// appropriate HTTP status (`400`, `401`, `405`, `408`, or `413`) to `stream`
/// and returns [`RequestOutcome::Handled`].
pub fn read_json_rpc_request<A>(
    stream: &TcpStream,
    authenticate: impl FnOnce(&str) -> Option<A>,
) -> std::io::Result<RequestOutcome<A>> {
    read_json_rpc_request_with_timeout(stream, DEFAULT_IO_TIMEOUT, authenticate)
}

/// Like [`read_json_rpc_request`], using an explicit read/write `timeout`.
pub fn read_json_rpc_request_with_timeout<A>(
    stream: &TcpStream,
    timeout: Duration,
    authenticate: impl FnOnce(&str) -> Option<A>,
) -> std::io::Result<RequestOutcome<A>> {
    configure_stream_timeouts(stream, timeout)?;
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    match reader.read_line(&mut request_line) {
        Ok(0) => {
            return respond_and_close(
                stream,
                &mut reader,
                400,
                "empty request",
                b"{\"error\":\"empty request\"}",
            );
        }
        Ok(_) => {}
        Err(e) if is_timeout_error(&e) => {
            log::warn!("mcp_http: request line read timed out: {e}");
            return respond_and_close(
                stream,
                &mut reader,
                408,
                "request line read timeout",
                b"{\"error\":\"request timeout\"}",
            );
        }
        Err(e) => {
            log::warn!("mcp_http: request line read error: {e}");
            return respond_and_close(
                stream,
                &mut reader,
                400,
                "request line read error",
                b"{\"error\":\"malformed request\"}",
            );
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    let has_extra = parts.next().is_some();
    let valid_request_line =
        !method.is_empty() && path.starts_with('/') && version.starts_with("HTTP/1.") && !has_extra;
    if !valid_request_line {
        return respond_and_close(
            stream,
            &mut reader,
            400,
            "malformed request line",
            b"{\"error\":\"malformed request line\"}",
        );
    }
    let is_post_mcp = method == "POST" && path == "/mcp";

    let mut content_length: Option<usize> = None;
    let mut malformed_header = false;
    let mut bearer = None;
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => {
                return respond_and_close(
                    stream,
                    &mut reader,
                    400,
                    "truncated headers",
                    b"{\"error\":\"truncated headers\"}",
                );
            }
            Ok(_) => {}
            Err(e) if is_timeout_error(&e) => {
                log::warn!("mcp_http: header read timed out: {e}");
                return respond_and_close(
                    stream,
                    &mut reader,
                    408,
                    "header read timeout",
                    b"{\"error\":\"request timeout\"}",
                );
            }
            Err(e) => {
                log::warn!("mcp_http: header read error: {e}");
                return respond_and_close(
                    stream,
                    &mut reader,
                    400,
                    "header read error",
                    b"{\"error\":\"malformed headers\"}",
                );
            }
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line ends headers
        }
        let Some((raw_name, raw_value)) = trimmed.split_once(':') else {
            malformed_header = true;
            continue;
        };
        if raw_name.trim().is_empty() || raw_name.chars().any(char::is_whitespace) {
            malformed_header = true;
            continue;
        }
        let value = raw_value.trim();
        if raw_name.eq_ignore_ascii_case("content-length") {
            match value.parse::<usize>() {
                Ok(len) => content_length = Some(len),
                Err(_) => malformed_header = true,
            }
        } else if raw_name.eq_ignore_ascii_case("authorization")
            && value
                .get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
        {
            let token = value.get(7..).map(str::trim).unwrap_or("");
            if !token.is_empty() {
                bearer = Some(token.to_string());
            }
        }
    }

    if malformed_header {
        return respond_and_close(
            stream,
            &mut reader,
            400,
            "malformed header",
            b"{\"error\":\"malformed header\"}",
        );
    }

    let content_length = content_length.unwrap_or(0);
    if !is_post_mcp {
        return respond_and_close(
            stream,
            &mut reader,
            405,
            "method not allowed",
            b"{\"error\":\"method not allowed\"}",
        );
    }
    if content_length == 0 {
        return respond_and_close(
            stream,
            &mut reader,
            400,
            "empty request body",
            b"{\"error\":\"empty request body\"}",
        );
    }
    if content_length > MAX_BODY {
        return respond_and_close(
            stream,
            &mut reader,
            413,
            "payload too large",
            b"{\"error\":\"payload too large\"}",
        );
    }

    let mut buf = vec![0u8; content_length];
    if let Err(e) = reader.read_exact(&mut buf) {
        if is_timeout_error(&e) {
            log::warn!("mcp_http: body read timed out: {e}");
            return respond_and_close(
                stream,
                &mut reader,
                408,
                "body read timeout",
                b"{\"error\":\"request timeout\"}",
            );
        }
        log::warn!("mcp_http: body read failed: {e}");
        return respond_and_close(
            stream,
            &mut reader,
            400,
            "truncated body",
            b"{\"error\":\"truncated body\"}",
        );
    }

    let body = match serde_json::from_slice(&buf) {
        Ok(body) => body,
        Err(e) => {
            log::warn!("mcp_http: invalid JSON body: {e}");
            return respond_and_close(
                stream,
                &mut reader,
                400,
                "invalid json",
                b"{\"error\":\"invalid json\"}",
            );
        }
    };

    let Some(auth) = bearer.as_deref().and_then(authenticate) else {
        return respond_and_close(
            stream,
            &mut reader,
            401,
            "unauthorized",
            b"{\"error\":\"unauthorized\"}",
        );
    };

    Ok(RequestOutcome::Json { body, auth })
}

/// Serve a single HTTP/1.1 MCP JSON-RPC connection on `stream`.
///
/// Configures socket read/write timeouts, parses and authenticates the request
/// (always writing an HTTP response on malformed/empty input, read timeout, or
/// auth failure), runs `dispatch` on a valid JSON-RPC body, and writes the
/// `200 OK` response.
pub fn handle_connection<A>(
    stream: TcpStream,
    authenticate: impl FnOnce(&str) -> Option<A>,
    dispatch: impl FnOnce(serde_json::Value, A) -> serde_json::Value,
) -> std::io::Result<()> {
    let (json, auth) = match read_json_rpc_request(&stream, authenticate)? {
        RequestOutcome::Json { body, auth } => (body, auth),
        RequestOutcome::Handled => return Ok(()),
    };
    let response_body = dispatch(json, auth);
    let body_bytes = serde_json::to_vec(&response_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut write_stream = &stream;
    let write_result = write_http_response(&mut write_stream, 200, &body_bytes);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    write_result
}

/// Like [`handle_connection`], using an explicit read/write `timeout`.
#[cfg(test)]
pub fn handle_connection_with_timeout<A>(
    stream: TcpStream,
    timeout: Duration,
    authenticate: impl FnOnce(&str) -> Option<A>,
    dispatch: impl FnOnce(serde_json::Value, A) -> serde_json::Value,
) -> std::io::Result<()> {
    let (json, auth) = match read_json_rpc_request_with_timeout(&stream, timeout, authenticate)? {
        RequestOutcome::Json { body, auth } => (body, auth),
        RequestOutcome::Handled => return Ok(()),
    };
    let response_body = dispatch(json, auth);
    let body_bytes = serde_json::to_vec(&response_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut write_stream = &stream;
    let write_result = write_http_response(&mut write_stream, 200, &body_bytes);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    write_result
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
        408 => "Request Timeout",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn parse_status_and_body(resp: &[u8]) -> (u16, serde_json::Value) {
        let text = String::from_utf8_lossy(resp);
        let status_line = text
            .lines()
            .next()
            .filter(|line| !line.is_empty())
            .unwrap_or_else(|| {
                panic!("mcp_http test: empty HTTP status line (raw bytes: {resp:?})")
            });
        let mut parts = status_line.split_whitespace();
        let version = parts.next().unwrap_or("");
        assert!(
            version.starts_with("HTTP/1."),
            "mcp_http test: unexpected HTTP version in {status_line:?} (raw bytes: {resp:?})"
        );
        let status: u16 = parts
            .next()
            .unwrap_or_else(|| {
                panic!(
                    "mcp_http test: missing status token in {status_line:?} (raw bytes: {resp:?})"
                )
            })
            .parse()
            .unwrap_or_else(|e| {
                panic!(
                    "mcp_http test: unparseable status in {status_line:?}: {e} (raw bytes: {resp:?})"
                )
            });
        let body_start = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or_else(|| {
                panic!("mcp_http test: missing header terminator in response (raw bytes: {resp:?})")
            });
        let json: serde_json::Value =
            serde_json::from_slice(&resp[body_start..]).unwrap_or_else(|e| {
                panic!(
                    "mcp_http test: invalid JSON body ({e}) in response (raw bytes: {:?})",
                    &resp[body_start..]
                )
            });
        (status, json)
    }

    fn run_single_connection(
        timeout: Duration,
        client_fn: impl FnOnce(TcpStream),
    ) -> (Option<Duration>, Option<Duration>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local_addr");
        let server_thread = std::thread::spawn(move || {
            let (server_stream, _) = listener.accept().expect("accept");
            let inspect = server_stream.try_clone().expect("clone server stream");
            handle_connection_with_timeout(
                server_stream,
                timeout,
                |token| (token == "valid-token").then_some("caller"),
                |body, _auth| serde_json::json!({ "ok": true, "echo": body }),
            )
            .expect("handle_connection_with_timeout");
            (
                inspect.read_timeout().expect("read_timeout"),
                inspect.write_timeout().expect("write_timeout"),
            )
        });
        let client = TcpStream::connect(addr).expect("connect to test server");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("client set_read_timeout");
        client
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("client set_write_timeout");
        client_fn(client);
        server_thread.join().expect("server thread join")
    }

    #[test]
    fn handle_connection_sets_read_and_write_timeouts_and_serves_valid_post() {
        let configured_timeout = Duration::from_millis(750);
        let mut resp = Vec::new();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let (read_to, write_to) = run_single_connection(configured_timeout, |mut client| {
            let req = format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer valid-token\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            client.write_all(req.as_bytes()).expect("write headers");
            client.write_all(body).expect("write body");
            client.flush().expect("flush");
            client
                .read_to_end(&mut resp)
                .expect("read_to_end valid response");
        });
        assert!(
            read_to.is_some(),
            "handle_connection must set a socket read timeout"
        );
        assert_eq!(
            write_to,
            Some(configured_timeout),
            "handle_connection must set the configured socket write timeout"
        );
        let (status, json) = parse_status_and_body(&resp);
        assert_eq!(status, 200);
        assert_eq!(json["ok"], true);
    }

    #[test]
    fn empty_request_writes_400_instead_of_closing_with_zero_bytes() {
        let mut resp = Vec::new();
        run_single_connection(Duration::from_secs(2), |mut client| {
            client
                .shutdown(std::net::Shutdown::Write)
                .expect("shutdown write on empty request");
            client
                .read_to_end(&mut resp)
                .expect("read_to_end on empty request");
        });
        assert!(
            !resp.is_empty(),
            "empty request must receive an HTTP 400 response, not a zero-byte close"
        );
        let (status, json) = parse_status_and_body(&resp);
        assert_eq!(status, 400);
        assert_eq!(json["error"], "empty request");
    }

    #[test]
    fn malformed_requests_always_write_400_response() {
        let cases: &[(&str, &[u8])] = &[
            ("blank request line", b"\r\n\r\n"),
            ("garbage request line", b"NOT_HTTP_AT_ALL\r\n\r\n"),
            (
                "missing http version",
                b"POST /mcp\r\nHost: localhost\r\n\r\n",
            ),
            (
                "truncated headers before blank line",
                b"POST /mcp HTTP/1.1\r\nHost: localhost\r\n",
            ),
            (
                "header without colon",
                b"POST /mcp HTTP/1.1\r\nMalformedHeaderLine\r\n\r\n",
            ),
            (
                "non-numeric content-length",
                b"POST /mcp HTTP/1.1\r\nContent-Length: not-a-number\r\n\r\n",
            ),
            (
                "zero content-length on POST /mcp",
                b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer valid-token\r\nContent-Length: 0\r\n\r\n",
            ),
            (
                "truncated body before content-length satisfied",
                b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer valid-token\r\nContent-Length: 50\r\n\r\n{\"short\":true}",
            ),
            (
                "invalid json body",
                b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer valid-token\r\nContent-Length: 9\r\n\r\n{not-json",
            ),
        ];

        for (label, payload) in cases {
            let mut resp = Vec::new();
            run_single_connection(Duration::from_secs(2), |mut client| {
                client.write_all(payload).expect("write malformed payload");
                client.flush().expect("flush malformed payload");
                client
                    .shutdown(std::net::Shutdown::Write)
                    .expect("shutdown write");
                client
                    .read_to_end(&mut resp)
                    .unwrap_or_else(|e| panic!("{label}: read_to_end failed: {e}"));
            });
            let (status, json) = parse_status_and_body(&resp);
            assert_eq!(status, 400, "{label}: expected 400, got {status} ({json})");
            assert!(
                json.get("error").and_then(|v| v.as_str()).is_some(),
                "{label}: expected error message in 400 body: {json}"
            );
        }
    }

    #[test]
    fn read_timeout_writes_408_at_each_request_stage() {
        let stages: &[(&str, &[u8])] = &[
            ("stalled before request line", b""),
            (
                "stalled mid-headers",
                b"POST /mcp HTTP/1.1\r\nHost: localhost\r\n",
            ),
            (
                "stalled mid-body",
                b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer valid-token\r\nContent-Length: 64\r\n\r\n{\"partial\":",
            ),
        ];

        for (label, partial_bytes) in stages {
            let mut resp = Vec::new();
            run_single_connection(Duration::from_millis(80), |mut client| {
                if !partial_bytes.is_empty() {
                    client
                        .write_all(partial_bytes)
                        .expect("write partial request");
                    client.flush().expect("flush partial request");
                }
                // Intentionally keep write half open so the server hits its read timeout.
                client
                    .read_to_end(&mut resp)
                    .unwrap_or_else(|e| panic!("{label}: read_to_end failed: {e}"));
            });
            let (status, json) = parse_status_and_body(&resp);
            assert_eq!(status, 408, "{label}: expected 408, got {status} ({json})");
            assert_eq!(json["error"], "request timeout", "{label}");
        }
    }
}
