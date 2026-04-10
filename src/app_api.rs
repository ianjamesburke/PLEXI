/// Structured API for app ↔ Plexi communication.
///
/// Apps send `ApiRequest` over stdin, Plexi validates against permissions
/// and scope, executes, and returns `ApiResponse` over stdout. Apps never
/// get raw filesystem or terminal access.

use crate::app_permissions::{AppPermissions, FsPermission, TrustLevel};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ─── Request / Response types ────────────────────────────────────────────────

/// API requests an app can send to Plexi.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiRequest {
    /// List directory contents. Requires filesystem >= ReadOnly.
    ListDir { path: String },
    /// Read a file's contents. Requires filesystem >= ReadOnly.
    /// Rejects .env files unless env_file_access is true.
    ReadFile { path: String },
    /// Write content to a file. Requires filesystem = ReadWrite.
    WriteFile { path: String, content: String },
    /// Get a secret from Keychain (scoped to app + directory, with walk-up).
    SecretGet { key: String },
    /// Store a secret in Keychain (scoped to app + directory).
    SecretStore { key: String, value: String },
    /// Request secure input from user (Plexi renders the masked field).
    SecureInput { prompt: String, mask: bool },
}

/// API responses from Plexi back to the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiResponse {
    DirListing { entries: Vec<DirEntry> },
    FileContent { content: String },
    WriteOk,
    Secret { value: Option<String> },
    SecretStored,
    /// The secure input result comes asynchronously after user interaction.
    SecureInputPending { request_id: String },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

// ─── Sensitive file detection ────────────────────────────────────────────────

/// Returns true if the filename matches a sensitive-file pattern that requires
/// `env_file_access` permission.
fn is_sensitive_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    lower.starts_with(".env")
        || lower.starts_with("credentials")
        || lower.ends_with(".key")
        || lower.ends_with(".pem")
}

// ─── Path resolution & scope check ──────────────────────────────────────────

/// Resolve a user-supplied path relative to scope_root, then verify it doesn't
/// escape scope. Returns the canonical path on success, or an error message.
fn resolve_and_check_scope(raw: &str, scope_root: &Path) -> Result<std::path::PathBuf, String> {
    let candidate = if Path::new(raw).is_absolute() {
        std::path::PathBuf::from(raw)
    } else {
        scope_root.join(raw)
    };

    // Canonicalize both to resolve symlinks and `..` components.
    let canonical = std::fs::canonicalize(&candidate).map_err(|e| {
        format!(
            "Cannot resolve path {}: {e}",
            candidate.display()
        )
    })?;
    let canonical_root = std::fs::canonicalize(scope_root).map_err(|e| {
        format!(
            "Cannot resolve scope root {}: {e}",
            scope_root.display()
        )
    })?;

    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "Path {} is outside app scope {}",
            canonical.display(),
            canonical_root.display()
        ));
    }

    Ok(canonical)
}

// ─── Handler ─────────────────────────────────────────────────────────────────

/// Process an API request, enforcing permissions and scope.
///
/// All filesystem paths are resolved relative to `scope_root`. Paths that
/// escape the scope directory are rejected for non-Builtin apps.
pub fn handle_api_request(
    request: &ApiRequest,
    app_id: &str,
    permissions: &AppPermissions,
    scope_root: &Path,
) -> ApiResponse {
    match request {
        ApiRequest::ListDir { path } => handle_list_dir(path, permissions, scope_root),
        ApiRequest::ReadFile { path } => handle_read_file(path, permissions, scope_root),
        ApiRequest::WriteFile { path, content } => {
            handle_write_file(path, content, permissions, scope_root)
        }
        ApiRequest::SecretGet { key } => handle_secret_get(key, app_id, scope_root),
        ApiRequest::SecretStore { key, value } => {
            handle_secret_store(key, value, app_id, scope_root)
        }
        ApiRequest::SecureInput { prompt, mask } => handle_secure_input(prompt, *mask),
    }
}

fn handle_list_dir(path: &str, perms: &AppPermissions, scope_root: &Path) -> ApiResponse {
    if perms.filesystem == FsPermission::None {
        return ApiResponse::Error {
            message: "App does not have filesystem read permission".to_string(),
        };
    }

    let resolved = match resolve_and_check_scope(path, scope_root) {
        Ok(p) => p,
        Err(msg) => return ApiResponse::Error { message: msg },
    };

    let read_dir = match std::fs::read_dir(&resolved) {
        Ok(rd) => rd,
        Err(e) => {
            return ApiResponse::Error {
                message: format!("Failed to list {}: {e}", resolved.display()),
            }
        }
    };

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().ok();
        entries.push(DirEntry {
            is_dir: metadata.as_ref().map_or(false, |m| m.is_dir()),
            size: metadata.as_ref().and_then(|m| {
                if m.is_file() {
                    Some(m.len())
                } else {
                    None
                }
            }),
            name,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    ApiResponse::DirListing { entries }
}

fn handle_read_file(path: &str, perms: &AppPermissions, scope_root: &Path) -> ApiResponse {
    if perms.filesystem == FsPermission::None {
        return ApiResponse::Error {
            message: "App does not have filesystem read permission".to_string(),
        };
    }

    let resolved = match resolve_and_check_scope(path, scope_root) {
        Ok(p) => p,
        Err(msg) => return ApiResponse::Error { message: msg },
    };

    // Check sensitive file patterns unless env_file_access is granted.
    if !perms.env_file_access {
        if let Some(filename) = resolved.file_name().and_then(|f| f.to_str()) {
            if is_sensitive_file(filename) {
                return ApiResponse::Error {
                    message: format!(
                        "Access denied: {filename} matches a sensitive file pattern. \
                         App needs env_file_access permission."
                    ),
                };
            }
        }
    }

    match std::fs::read_to_string(&resolved) {
        Ok(content) => ApiResponse::FileContent { content },
        Err(e) => ApiResponse::Error {
            message: format!("Failed to read {}: {e}", resolved.display()),
        },
    }
}

fn handle_write_file(
    path: &str,
    content: &str,
    perms: &AppPermissions,
    scope_root: &Path,
) -> ApiResponse {
    if perms.filesystem != FsPermission::ReadWrite {
        return ApiResponse::Error {
            message: "App does not have filesystem write permission".to_string(),
        };
    }

    // For writes, the file may not exist yet. Resolve the parent directory
    // against scope, then append the filename.
    let candidate = if Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        scope_root.join(path)
    };

    let parent = match candidate.parent() {
        Some(p) => p,
        None => {
            return ApiResponse::Error {
                message: "Invalid file path (no parent directory)".to_string(),
            }
        }
    };

    let canonical_parent = match std::fs::canonicalize(parent) {
        Ok(p) => p,
        Err(e) => {
            return ApiResponse::Error {
                message: format!("Cannot resolve parent directory {}: {e}", parent.display()),
            }
        }
    };

    let canonical_root = match std::fs::canonicalize(scope_root) {
        Ok(p) => p,
        Err(e) => {
            return ApiResponse::Error {
                message: format!("Cannot resolve scope root: {e}"),
            }
        }
    };

    if !canonical_parent.starts_with(&canonical_root) {
        return ApiResponse::Error {
            message: format!(
                "Path {} is outside app scope {}",
                candidate.display(),
                canonical_root.display()
            ),
        };
    }

    let final_path = canonical_parent.join(candidate.file_name().unwrap_or_default());

    match std::fs::write(&final_path, content) {
        Ok(()) => ApiResponse::WriteOk,
        Err(e) => ApiResponse::Error {
            message: format!("Failed to write {}: {e}", final_path.display()),
        },
    }
}

fn handle_secret_get(key: &str, app_id: &str, scope_root: &Path) -> ApiResponse {
    let dir_str = scope_root.display().to_string();
    let value = crate::secrets::resolve_secret(key, app_id, &dir_str);
    ApiResponse::Secret { value }
}

fn handle_secret_store(key: &str, value: &str, app_id: &str, scope_root: &Path) -> ApiResponse {
    let dir_str = scope_root.display().to_string();
    if crate::secrets::store_secret(key, value, app_id, &dir_str) {
        ApiResponse::SecretStored
    } else {
        ApiResponse::Error {
            message: format!("Failed to store secret '{key}'"),
        }
    }
}

fn handle_secure_input(_prompt: &str, _mask: bool) -> ApiResponse {
    // Secure input is handled by the UI layer. We return a pending token;
    // the actual value is delivered asynchronously via a separate channel.
    let request_id = format!("sec_{}", uuid_v4_simple());
    ApiResponse::SecureInputPending { request_id }
}

/// Minimal v4-style random ID (no external dep required).
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Not cryptographically random, but sufficient for request correlation.
    // Replace with a proper UUID crate if one is already in the dependency tree.
    format!("{:016x}", nanos)
}
