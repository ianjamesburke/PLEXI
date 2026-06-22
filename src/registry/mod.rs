//! v2 WASM registry client primitives.

pub mod payment;
pub mod stub;
pub mod verify;

use crate::registry::payment::{PaymentRequired, PaymentSession};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_REGISTRY_BASE_URL: &str = "https://registry.plexiapp.com";
pub const DEFAULT_CDN_BASE_URL: &str = "https://cdn.plexiapp.com";
pub const CPYTHON_RUNTIME_ALIAS: &str = "@plexi/cpython-runtime@3.12";

#[derive(Debug, thiserror::Error)]
pub enum RegistryClientError {
    #[error("invalid registry name '{0}'")]
    InvalidName(String),
    #[error("registry request failed for {url}: {message}")]
    Request { url: String, message: String },
    #[error("registry response parse failed for {url}: {message}")]
    Parse { url: String, message: String },
    #[error("bundle hash mismatch for {path}: expected {expected}, actual {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("io error during {action} at {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("payment required: {0:?}")]
    PaymentRequired(PaymentRequired),
    #[error(
        "payment declined for {endpoint}: price {price_usd_cents} cents exceeds auto-pay threshold {threshold_cents} cents"
    )]
    PaymentDeclined {
        endpoint: String,
        price_usd_cents: u64,
        threshold_cents: u64,
    },
    #[error("publisher key missing for '{0}'")]
    PublisherKeyMissing(String),
    #[error("bundle signature missing for '{0}'")]
    SignatureMissing(String),
    #[error("signature decode failed for {field}: {message}")]
    SignatureDecode {
        field: &'static str,
        message: String,
    },
    #[error("bundle signature verification failed: {0}")]
    Verify(#[from] verify::VerifyError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryIndex {
    pub latest: String,
    #[serde(default)]
    pub versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryManifest {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub hash: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub trust_tier: TrustTier,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub optional_capabilities: Vec<String>,
    #[serde(default)]
    pub python_compat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublisherKey {
    pub publisher: String,
    pub ed25519_public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRegistryApp {
    pub requested: String,
    pub manifest: RegistryManifest,
    pub bundle_path: PathBuf,
    pub publisher_key: PublisherKey,
    pub payment: Option<PaymentRequired>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TrustTier {
    #[default]
    Unverified,
    Verified,
    Curated,
}

pub trait HttpTransport {
    fn get(&self, url: &str, bearer: Option<&str>) -> Result<HttpResponse, String>;
    fn post(&self, url: &str, body: &[u8], bearer: Option<&str>) -> Result<HttpResponse, String>;
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct UreqTransport;

impl HttpTransport for UreqTransport {
    fn get(&self, url: &str, bearer: Option<&str>) -> Result<HttpResponse, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build();
        let mut req = agent.get(url);
        if let Some(token) = bearer {
            req = req.set("authorization", &format!("Bearer {token}"));
        }
        match req.call() {
            Ok(resp) => {
                let status = resp.status();
                let mut body = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| e.to_string())?;
                Ok(HttpResponse { status, body })
            }
            Err(ureq::Error::Status(status, resp)) => {
                let mut body = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| e.to_string())?;
                Ok(HttpResponse { status, body })
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn post(&self, url: &str, body: &[u8], bearer: Option<&str>) -> Result<HttpResponse, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build();
        let mut req = agent.post(url);
        if let Some(token) = bearer {
            req = req.set("authorization", &format!("Bearer {token}"));
        }
        match req.send_bytes(body) {
            Ok(resp) => {
                let status = resp.status();
                let mut body = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| e.to_string())?;
                Ok(HttpResponse { status, body })
            }
            Err(ureq::Error::Status(status, resp)) => {
                let mut body = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| e.to_string())?;
                Ok(HttpResponse { status, body })
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

pub struct RegistryClient<T = UreqTransport> {
    registry_base: String,
    cdn_base: String,
    cache_dir: PathBuf,
    transport: T,
}

impl RegistryClient<UreqTransport> {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self::with_transport(
            DEFAULT_REGISTRY_BASE_URL.to_string(),
            DEFAULT_CDN_BASE_URL.to_string(),
            cache_dir,
            UreqTransport,
        )
    }
}

impl<T: HttpTransport> RegistryClient<T> {
    pub fn with_transport(
        registry_base: String,
        cdn_base: String,
        cache_dir: PathBuf,
        transport: T,
    ) -> Self {
        Self {
            registry_base,
            cdn_base,
            cache_dir,
            transport,
        }
    }

    pub fn prepare(
        &self,
        name: &str,
        auto_pay_threshold_cents: u64,
    ) -> Result<PreparedRegistryApp, RegistryClientError> {
        let (manifest, bearer, payment) =
            self.resolve_with_payment(name, auto_pay_threshold_cents)?;
        let dest = self.cache_path(&manifest.hash);
        let bundle_path = self.fetch_bundle_authorized(
            &manifest.hash,
            &dest,
            bearer.as_deref(),
            auto_pay_threshold_cents,
        )?;
        let publisher_key = self.fetch_publisher_key(&manifest.publisher)?;
        let signature =
            decode_hex_field(&manifest.signature, "manifest.signature").and_then(|sig| {
                if sig.is_empty() {
                    Err(RegistryClientError::SignatureMissing(manifest.id.clone()))
                } else {
                    Ok(sig)
                }
            })?;
        let public_key = decode_hex_field(
            &publisher_key.ed25519_public_key,
            "publisher.ed25519_public_key",
        )?;
        verify::verify_bundle(&bundle_path, &signature, &public_key)?;
        log::info!(
            "registry: prepared app={} hash={} bundle={} payment={}",
            manifest.id,
            manifest.hash,
            bundle_path.display(),
            payment.is_some()
        );
        Ok(PreparedRegistryApp {
            requested: name.to_string(),
            manifest,
            bundle_path,
            publisher_key,
            payment,
        })
    }

    pub fn resolve(&self, name: &str) -> Result<RegistryManifest, RegistryClientError> {
        self.resolve_with_payment(name, 0)
            .map(|(manifest, _, _)| manifest)
    }

    fn resolve_with_payment(
        &self,
        name: &str,
        auto_pay_threshold_cents: u64,
    ) -> Result<(RegistryManifest, Option<String>, Option<PaymentRequired>), RegistryClientError>
    {
        let (alias, version) = parse_registry_name(name)?;
        let index_url = format!("{}/index/{}", self.registry_base, alias);
        log::info!("registry: resolving {name} via {index_url}");
        let index_resp = self.get_ok(&index_url, None)?;
        let index: RegistryIndex =
            serde_json::from_slice(&index_resp.body).map_err(|e| RegistryClientError::Parse {
                url: index_url.clone(),
                message: e.to_string(),
            })?;
        let hash = match version {
            Some(version) => index
                .versions
                .get(version)
                .cloned()
                .ok_or_else(|| RegistryClientError::InvalidName(name.to_string()))?,
            None => index.latest,
        };
        let manifest_url = format!("{}/manifests/{}.toml", self.registry_base, hash);
        let (manifest_resp, bearer, payment) =
            self.get_ok_with_payment(&manifest_url, auto_pay_threshold_cents)?;
        let manifest_text =
            std::str::from_utf8(&manifest_resp.body).map_err(|e| RegistryClientError::Parse {
                url: manifest_url.clone(),
                message: e.to_string(),
            })?;
        let manifest: RegistryManifest =
            toml::from_str(manifest_text).map_err(|e| RegistryClientError::Parse {
                url: manifest_url.clone(),
                message: e.to_string(),
            })?;
        log::info!("registry: resolved {name} to hash={}", manifest.hash);
        Ok((manifest, bearer, payment))
    }

    pub fn fetch_bundle(&self, hash: &str, dest: &Path) -> Result<PathBuf, RegistryClientError> {
        self.fetch_bundle_authorized(hash, dest, None, 0)
    }

    fn fetch_bundle_authorized(
        &self,
        hash: &str,
        dest: &Path,
        bearer: Option<&str>,
        auto_pay_threshold_cents: u64,
    ) -> Result<PathBuf, RegistryClientError> {
        if dest.exists() && sha256_file(dest)? == hash {
            log::info!(
                "registry: bundle cache hit hash={} path={}",
                hash,
                dest.display()
            );
            return Ok(dest.to_path_buf());
        }
        let url = format!("{}/bundles/{}.wasm", self.cdn_base, hash);
        log::info!("registry: fetching bundle hash={} from {url}", hash);
        let response =
            self.transport
                .get(&url, bearer)
                .map_err(|message| RegistryClientError::Request {
                    url: url.clone(),
                    message,
                })?;
        if response.status == 402 {
            let payment = payment::parse_payment_required(&response.body).map_err(|message| {
                RegistryClientError::Parse {
                    url: url.clone(),
                    message,
                }
            })?;
            let session = self.pay(&payment, auto_pay_threshold_cents)?;
            return self.fetch_bundle_authorized(
                hash,
                dest,
                Some(&session.session_jwt),
                auto_pay_threshold_cents,
            );
        }
        if response.status >= 400 {
            return Err(RegistryClientError::Request {
                url,
                message: format!("http {}", response.status),
            });
        }
        write_verified_bundle(dest, hash, &response.body)?;
        Ok(dest.to_path_buf())
    }

    pub fn cache_path(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{hash}.wasm"))
    }

    fn fetch_publisher_key(&self, publisher: &str) -> Result<PublisherKey, RegistryClientError> {
        let url = format!("{}/publishers/{}.json", self.registry_base, publisher);
        let response = self.get_ok(&url, None)?;
        let key: PublisherKey =
            serde_json::from_slice(&response.body).map_err(|e| RegistryClientError::Parse {
                url: url.clone(),
                message: e.to_string(),
            })?;
        if key.ed25519_public_key.is_empty() {
            return Err(RegistryClientError::PublisherKeyMissing(
                publisher.to_string(),
            ));
        }
        Ok(key)
    }

    fn get_ok(
        &self,
        url: &str,
        _bearer: Option<&str>,
    ) -> Result<HttpResponse, RegistryClientError> {
        self.get_ok_with_payment(url, 0)
            .map(|(response, _, _)| response)
    }

    fn get_ok_with_payment(
        &self,
        url: &str,
        auto_pay_threshold_cents: u64,
    ) -> Result<(HttpResponse, Option<String>, Option<PaymentRequired>), RegistryClientError> {
        let response =
            self.transport
                .get(url, None)
                .map_err(|message| RegistryClientError::Request {
                    url: url.to_string(),
                    message,
                })?;
        if response.status == 402 {
            let payment = payment::parse_payment_required(&response.body).map_err(|message| {
                RegistryClientError::Parse {
                    url: url.to_string(),
                    message,
                }
            })?;
            let session = self.pay(&payment, auto_pay_threshold_cents)?;
            let retry = self
                .transport
                .get(url, Some(&session.session_jwt))
                .map_err(|message| RegistryClientError::Request {
                    url: url.to_string(),
                    message,
                })?;
            if retry.status >= 400 {
                return Err(RegistryClientError::Request {
                    url: url.to_string(),
                    message: format!("http {}", retry.status),
                });
            }
            return Ok((retry, Some(session.session_jwt), Some(payment)));
        }
        if response.status >= 400 {
            return Err(RegistryClientError::Request {
                url: url.to_string(),
                message: format!("http {}", response.status),
            });
        }
        Ok((response, None, None))
    }

    fn pay(
        &self,
        payment: &PaymentRequired,
        auto_pay_threshold_cents: u64,
    ) -> Result<PaymentSession, RegistryClientError> {
        if !payment::should_auto_pay(payment.price_usd_cents, auto_pay_threshold_cents) {
            return Err(RegistryClientError::PaymentDeclined {
                endpoint: payment.payment_endpoint.clone(),
                price_usd_cents: payment.price_usd_cents,
                threshold_cents: auto_pay_threshold_cents,
            });
        }
        log::info!(
            "registry: auto-paying {} cents via {}",
            payment.price_usd_cents,
            payment.payment_endpoint
        );
        let response = self
            .transport
            .post(&payment.payment_endpoint, b"{}", None)
            .map_err(|message| RegistryClientError::Request {
                url: payment.payment_endpoint.clone(),
                message,
            })?;
        if response.status >= 400 {
            return Err(RegistryClientError::Request {
                url: payment.payment_endpoint.clone(),
                message: format!("http {}", response.status),
            });
        }
        serde_json::from_slice(&response.body).map_err(|e| RegistryClientError::Parse {
            url: payment.payment_endpoint.clone(),
            message: e.to_string(),
        })
    }
}

fn parse_registry_name(name: &str) -> Result<(&str, Option<&str>), RegistryClientError> {
    if !name.starts_with('@') || !name.contains('/') {
        return Err(RegistryClientError::InvalidName(name.to_string()));
    }
    let slash = name
        .find('/')
        .ok_or_else(|| RegistryClientError::InvalidName(name.to_string()))?;
    let version_marker = name[slash + 1..].rfind('@').map(|idx| slash + 1 + idx);
    match version_marker {
        Some(idx) => Ok((&name[..idx], Some(&name[idx + 1..]))),
        None => Ok((name, None)),
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex_field(value: &str, field: &'static str) -> Result<Vec<u8>, RegistryClientError> {
    if value.len() % 2 != 0 {
        return Err(RegistryClientError::SignatureDecode {
            field,
            message: "hex string has odd length".to_string(),
        });
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text =
                std::str::from_utf8(pair).map_err(|e| RegistryClientError::SignatureDecode {
                    field,
                    message: e.to_string(),
                })?;
            u8::from_str_radix(text, 16).map_err(|e| RegistryClientError::SignatureDecode {
                field,
                message: e.to_string(),
            })
        })
        .collect()
}

fn write_verified_bundle(
    dest: &Path,
    expected_hash: &str,
    body: &[u8],
) -> Result<(), RegistryClientError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| RegistryClientError::Io {
            action: "create_dir_all",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let tmp = dest.with_extension("download");
    {
        let mut file = fs::File::create(&tmp).map_err(|source| RegistryClientError::Io {
            action: "create",
            path: tmp.clone(),
            source,
        })?;
        file.write_all(body)
            .map_err(|source| RegistryClientError::Io {
                action: "write",
                path: tmp.clone(),
                source,
            })?;
    }
    let actual = sha256_file(&tmp)?;
    if actual != expected_hash {
        let _ = fs::remove_file(&tmp);
        return Err(RegistryClientError::HashMismatch {
            path: tmp,
            expected: expected_hash.to_string(),
            actual,
        });
    }
    fs::rename(&tmp, dest).map_err(|source| RegistryClientError::Io {
        action: "rename",
        path: dest.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, RegistryClientError> {
    let mut file = fs::File::open(path).map_err(|source| RegistryClientError::Io {
        action: "open",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|source| RegistryClientError::Io {
        action: "hash",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MockTransport {
        responses: HashMap<String, HttpResponse>,
    }

    impl HttpTransport for MockTransport {
        fn get(&self, url: &str, _bearer: Option<&str>) -> Result<HttpResponse, String> {
            self.responses
                .get(url)
                .map(|r| HttpResponse {
                    status: r.status,
                    body: r.body.clone(),
                })
                .ok_or_else(|| format!("missing mock response for {url}"))
        }

        fn post(
            &self,
            url: &str,
            _body: &[u8],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse, String> {
            self.get(url, None)
        }
    }

    #[test]
    fn resolve_fetches_index_then_manifest() {
        let mut transport = MockTransport::default();
        transport.responses.insert(
            "https://reg/index/@test/hello".into(),
            HttpResponse {
                status: 200,
                body: br#"{"latest":"abc","versions":{"1.0.0":"abc"}}"#.to_vec(),
            },
        );
        transport.responses.insert(
            "https://reg/manifests/abc.toml".into(),
            HttpResponse { status: 200, body: b"id='com.test.hello'\nname='Hello'\npublisher='test'\nversion='1.0.0'\nhash='abc'\ntrust_tier='verified'\n".to_vec() },
        );
        let client = RegistryClient::with_transport(
            "https://reg".into(),
            "https://cdn".into(),
            PathBuf::new(),
            transport,
        );
        let manifest = client.resolve("@test/hello").unwrap();
        assert_eq!(manifest.id, "com.test.hello");
        assert_eq!(manifest.trust_tier, TrustTier::Verified);
    }

    #[test]
    fn fetch_bundle_writes_and_verifies_hash() {
        let body = b"wasm bytes";
        let hash = format!("{:x}", Sha256::digest(body));
        let mut transport = MockTransport::default();
        transport.responses.insert(
            format!("https://cdn/bundles/{hash}.wasm"),
            HttpResponse {
                status: 200,
                body: body.to_vec(),
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.wasm");
        let client = RegistryClient::with_transport(
            "https://reg".into(),
            "https://cdn".into(),
            dir.path().to_path_buf(),
            transport,
        );
        client.fetch_bundle(&hash, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
    }

    #[test]
    fn fetch_bundle_declines_payment_without_threshold() {
        let hash = "abc";
        let mut transport = MockTransport::default();
        transport.responses.insert(
            "https://cdn/bundles/abc.wasm".into(),
            HttpResponse {
                status: 402,
                body:
                    br#"{"price_usd_cents":25,"model":"per-run","payment_endpoint":"https://pay"}"#
                        .to_vec(),
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let client = RegistryClient::with_transport(
            "https://reg".into(),
            "https://cdn".into(),
            dir.path().to_path_buf(),
            transport,
        );
        let err = client
            .fetch_bundle(hash, &dir.path().join("x.wasm"))
            .unwrap_err();
        assert!(matches!(err, RegistryClientError::PaymentDeclined { .. }));
    }

    #[test]
    fn prepare_retries_payment_and_verifies_signature() {
        let addr = stub::spawn_for_test(true).unwrap();
        let base = format!("http://{addr}");
        let dir = tempfile::tempdir().unwrap();
        let client = RegistryClient::with_transport(
            base.clone(),
            base,
            dir.path().join("cache"),
            UreqTransport,
        );

        let prepared = client.prepare("@mock/paid-tool", 25).unwrap();

        assert_eq!(prepared.manifest.id, "com.mock.paid-tool");
        assert!(prepared.payment.is_some());
        assert!(prepared.bundle_path.exists());
    }
}
