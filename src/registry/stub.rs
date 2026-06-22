use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const BUNDLE: &[u8] = include_bytes!("../../tests/wasm-fixtures/counter.wasm");

pub fn serve_blocking(port: u16, paid: bool) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    println!("Registry stub listening on http://{addr}");
    println!("Config:");
    println!("[marketplace]");
    println!("wasm_registry_url = \"http://{addr}\"");
    println!("wasm_cdn_url = \"http://{addr}\"");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    serve_listener(listener, paid, None)
}

pub fn serve_listener(
    listener: TcpListener,
    paid: bool,
    max_requests: Option<usize>,
) -> Result<(), String> {
    let mut handled = 0usize;
    for stream in listener.incoming() {
        let stream = stream.map_err(|e| e.to_string())?;
        handle_stream(stream, paid)?;
        handled += 1;
        if max_requests.is_some_and(|max| handled >= max) {
            break;
        }
    }
    Ok(())
}

pub fn bundle_hash() -> String {
    crate::registry::hex_encode(&Sha256::digest(BUNDLE))
}

pub fn publisher_key_hex() -> String {
    crate::registry::hex_encode(&signing_key().verifying_key().to_bytes())
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

fn signature_hex() -> String {
    crate::registry::hex_encode(&signing_key().sign(BUNDLE).to_bytes())
}

fn handle_stream(mut stream: TcpStream, paid: bool) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let addr = stream.local_addr().map_err(|e| e.to_string())?;
    let request = read_request(&mut stream)?;
    let response = route(&request, paid, addr);
    stream
        .write_all(&response)
        .and_then(|_| stream.flush())
        .map_err(|e| e.to_string())
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = buf
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "malformed HTTP request".to_string())?;
    let header = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "missing HTTP request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_string())?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| "missing HTTP path".to_string())?
        .to_string();
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_string());
        }
    }
    Ok(HttpRequest {
        method,
        path,
        authorization,
    })
}

struct HttpRequest {
    method: String,
    path: String,
    authorization: Option<String>,
}

fn route(request: &HttpRequest, paid: bool, addr: SocketAddr) -> Vec<u8> {
    let hash = bundle_hash();
    if request.method == "GET" && request.path == "/index/@mock/paid-tool" {
        let body = format!(r#"{{"latest":"{hash}","versions":{{"1.0.0":"{hash}"}}}}"#);
        return json_response(200, body.as_bytes());
    }
    if request.method == "GET" && request.path == format!("/manifests/{hash}.toml") {
        if paid && !authorized(request) {
            return payment_required(addr);
        }
        let body = format!(
            "id='com.mock.paid-tool'\n\
             name='Paid Tool'\n\
             publisher='mock'\n\
             version='1.0.0'\n\
             hash='{hash}'\n\
             signature='{}'\n\
             trust_tier='verified'\n\
             required_capabilities=[]\n\
             optional_capabilities=[]\n",
            signature_hex()
        );
        return response(200, "application/toml", body.as_bytes());
    }
    if request.method == "GET" && request.path == format!("/bundles/{hash}.wasm") {
        if paid && !authorized(request) {
            return payment_required(addr);
        }
        return response(200, "application/wasm", BUNDLE);
    }
    if request.method == "GET" && request.path == "/publishers/mock.json" {
        let body = format!(
            r#"{{"publisher":"mock","ed25519_public_key":"{}"}}"#,
            publisher_key_hex()
        );
        return json_response(200, body.as_bytes());
    }
    if request.method == "POST" && request.path == "/payments/mock/paid-tool" {
        return json_response(
            200,
            br#"{"session_jwt":"stub-session-token","subscription":false}"#,
        );
    }
    json_response(404, br#"{"error":"not found"}"#)
}

fn authorized(request: &HttpRequest) -> bool {
    request.authorization.as_deref() == Some("Bearer stub-session-token")
}

fn payment_required(addr: SocketAddr) -> Vec<u8> {
    let body = format!(
        r#"{{"price_usd_cents":25,"model":"per-run","payment_endpoint":"http://{addr}/payments/mock/paid-tool"}}"#
    );
    json_response(402, body.as_bytes())
}

fn json_response(status: u16, body: &[u8]) -> Vec<u8> {
    response(status, "application/json", body)
}

fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        402 => "Payment Required",
        404 => "Not Found",
        _ => "OK",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[cfg(test)]
pub fn spawn_for_test(paid: bool) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = serve_listener(listener, paid, None);
    });
    Ok(addr)
}
