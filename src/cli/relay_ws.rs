//! Minimal WebSocket client for the phone relay.
//!
//! `ws://` is for a local relay. `wss://` is the production path, where TLS
//! ends at the relay (or at the platform proxy in front of it). Frame bodies
//! are not logged by this module.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct RelayUrl {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_relay_url(input: &str) -> Result<RelayUrl, String> {
    let parsed = url::Url::parse(input).map_err(|error| format!("relay url: {error}"))?;
    let tls = match parsed.scheme() {
        "ws" => false,
        "wss" => true,
        other => return Err(format!("relay url scheme `{other}` is not ws or wss")),
    };
    let host = parsed
        .host_str()
        .ok_or_else(|| "relay url is missing a host".to_string())?
        .to_string();
    if !tls && !is_loopback(&host) {
        return Err(
            "relay url must use wss:// (ws:// is only allowed for localhost)".to_string(),
        );
    }
    let port = parsed.port().unwrap_or(if tls { 443 } else { 80 });
    let mut path = parsed.path().to_string();
    if path.is_empty() || path == "/" {
        path = "/v1/desktop".to_string();
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(RelayUrl {
        tls,
        host,
        port,
        path,
    })
}

enum Io {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for Io {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Io::Plain(stream) => stream.read(buf),
            Io::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Io {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Io::Plain(stream) => stream.write(buf),
            Io::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Io::Plain(stream) => stream.flush(),
            Io::Tls(stream) => stream.flush(),
        }
    }
}

pub fn is_loopback(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

pub struct WsConn {
    io: Io,
    buf: Vec<u8>,
}

pub enum Incoming {
    Text(String),
    Timeout,
    Closed,
}

impl WsConn {
    pub fn connect(url: &RelayUrl) -> Result<Self, String> {
        let addr = format!("{}:{}", url.host, url.port);
        let mut targets = addr
            .to_socket_addrs()
            .map_err(|error| format!("relay dns {addr}: {error}"))?;
        let target = targets
            .next()
            .ok_or_else(|| format!("relay dns {addr} returned no address"))?;
        let stream = TcpStream::connect_timeout(&target, Duration::from_secs(10))
            .map_err(|error| format!("relay connect {addr}: {error}"))?;
        stream
            .set_nodelay(true)
            .map_err(|error| format!("relay nodelay: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("relay timeout: {error}"))?;
        let mut io = if url.tls {
            Io::Tls(Box::new(tls_wrap(stream, &url.host)?))
        } else {
            Io::Plain(stream)
        };
        let key = base64_std(&uuid::Uuid::new_v4().into_bytes());
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            path = url.path,
            host = url.host,
            port = url.port,
        );
        io.write_all(request.as_bytes())
            .map_err(|error| format!("relay handshake write: {error}"))?;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = io
                .read(&mut chunk)
                .map_err(|error| format!("relay handshake read: {error}"))?;
            if n == 0 {
                return Err("relay closed during websocket handshake".to_string());
            }
            raw.extend_from_slice(&chunk[..n]);
            if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            if raw.len() > 16 * 1024 {
                return Err("relay handshake response is too large".to_string());
            }
        }
        let header_end = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| "relay handshake is truncated".to_string())?;
        let header = String::from_utf8_lossy(&raw[..header_end]);
        let status = header.lines().next().unwrap_or("");
        if !status.contains("101") {
            return Err(format!("relay websocket upgrade failed: {status}"));
        }
        let accept = ws_accept(&key);
        if !header.contains(&accept) {
            return Err("relay websocket accept mismatch".to_string());
        }
        set_idle_timeout(&mut io)?;
        let buf = raw[header_end + 4..].to_vec();
        Ok(WsConn { io, buf })
    }

    pub fn send_text(&mut self, payload: &[u8]) -> Result<(), String> {
        self.send_frame(0x1, payload)
    }

    pub fn recv(&mut self) -> Result<Incoming, String> {
        loop {
            if let Some(frame) = self.pop_frame()? {
                match frame.opcode {
                    0x1 => {
                        let text = String::from_utf8(frame.payload)
                            .map_err(|_| "relay frame is not utf-8".to_string())?;
                        return Ok(Incoming::Text(text));
                    }
                    0x8 => return Ok(Incoming::Closed),
                    0x9 => self.send_frame(0xA, &frame.payload)?,
                    0xA => continue,
                    _ => continue,
                }
            }
            let mut chunk = [0u8; 4096];
            match self.io.read(&mut chunk) {
                Ok(0) => return Ok(Incoming::Closed),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(error) if is_timeout(&error) => {
                    if self.buf.is_empty() {
                        return Ok(Incoming::Timeout);
                    }
                }
                Err(error) => return Err(format!("relay read: {error}")),
            }
        }
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mask = uuid::Uuid::new_v4().into_bytes();
        let mask = [mask[0], mask[1], mask[2], mask[3]];
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ mask[i % 4]),
        );
        self.io
            .write_all(&frame)
            .map_err(|error| format!("relay write: {error}"))?;
        Ok(())
    }

    fn pop_frame(&mut self) -> Result<Option<Frame>, String> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let opcode = self.buf[0] & 0x0F;
        let masked = self.buf[1] & 0x80 != 0;
        let mut length = (self.buf[1] & 0x7F) as usize;
        let mut offset = 2;
        if length == 126 {
            if self.buf.len() < 4 {
                return Ok(None);
            }
            length = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
            offset = 4;
        } else if length == 127 {
            if self.buf.len() < 10 {
                return Ok(None);
            }
            let mut wide = [0u8; 8];
            wide.copy_from_slice(&self.buf[2..10]);
            length = u64::from_be_bytes(wide) as usize;
            offset = 10;
        }
        if length > 1024 * 1024 {
            return Err("relay frame is too large".to_string());
        }
        let mask_len = if masked { 4 } else { 0 };
        if self.buf.len() < offset + mask_len + length {
            return Ok(None);
        }
        let mask = if masked {
            let mut key = [0u8; 4];
            key.copy_from_slice(&self.buf[offset..offset + 4]);
            offset += 4;
            Some(key)
        } else {
            None
        };
        let mut payload = self.buf[offset..offset + length].to_vec();
        if let Some(key) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= key[i % 4];
            }
        }
        self.buf.drain(..offset + length);
        Ok(Some(Frame { opcode, payload }))
    }
}

struct Frame {
    opcode: u8,
    payload: Vec<u8>,
}

fn set_idle_timeout(io: &mut Io) -> Result<(), String> {
    let timeout = Some(Duration::from_millis(200));
    match io {
        Io::Plain(stream) => stream
            .set_read_timeout(timeout)
            .map_err(|error| format!("relay timeout: {error}")),
        Io::Tls(stream) => stream
            .sock
            .set_read_timeout(timeout)
            .map_err(|error| format!("relay timeout: {error}")),
    }
}

fn tls_wrap(
    stream: TcpStream,
    host: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| format!("relay tls name `{host}` is not a DNS name"))?;
    let conn = ClientConnection::new(Arc::new(config), name)
        .map_err(|error| format!("relay tls: {error}"))?;
    Ok(StreamOwned::new(conn, stream))
}

fn ws_accept(key: &str) -> String {
    let digest = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        format!("{key}{WS_GUID}").as_bytes(),
    );
    base64_std(digest.as_ref())
}

fn base64_std(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::parse_relay_url;

    #[test]
    fn parses_local_ws_and_production_wss() {
        let local = parse_relay_url("ws://127.0.0.1:8790/v1/desktop").unwrap();
        assert!(!local.tls);
        assert_eq!(local.port, 8790);
        assert_eq!(local.path, "/v1/desktop");
        let remote = parse_relay_url("wss://relay.example.up.railway.app").unwrap();
        assert!(remote.tls);
        assert_eq!(remote.port, 443);
        assert_eq!(remote.path, "/v1/desktop");
        assert!(parse_relay_url("ws://localhost:9/v1/desktop").is_ok());
        assert!(parse_relay_url("ws://[::1]:9/v1/desktop").is_ok());
        assert!(parse_relay_url("ws://example.com/v1/desktop").is_err());
        assert!(parse_relay_url("ws://10.0.0.8:8790/v1/desktop").is_err());
    }
}
