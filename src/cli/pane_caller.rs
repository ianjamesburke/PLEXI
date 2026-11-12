//! Whether this CLI process is an agent pane that must not release another
//! folder's secrets.
//!
//! `PLEXI_PANE_ID` is the direct signal. Clearing it is not enough: a child of
//! the pane's shell still has that shell as an ancestor, and the ancestor's
//! environment still names the pane and this host's socket. A pane of a
//! different host (a different `PLEXI_SOCKET`) does not count — the check is
//! scoped to the socket this process would talk to.

use std::path::Path;

/// `true` when this process is an agent pane, or a child of one on this host.
pub(crate) fn caller_is_pane_agent() -> bool {
    if own_pane_id().is_some() {
        return true;
    }
    let Some(socket) = resolved_socket() else {
        return false;
    };
    ancestor_is_pane_of_socket(&socket)
}

fn own_pane_id() -> Option<String> {
    let value = std::env::var("PLEXI_PANE_ID").ok()?;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn resolved_socket() -> Option<String> {
    super::resolve_command_socket().map(|path| path.to_string_lossy().into_owned())
}

fn ancestor_is_pane_of_socket(socket: &str) -> bool {
    let mut pid = std::process::id();
    for _ in 0..32 {
        let Some(parent) = crate::host::shell::get_pid_ppid(pid) else {
            break;
        };
        if parent == 0 || parent == 1 || parent == pid {
            break;
        }
        if process_environ_is_pane_of_socket(parent, socket) {
            return true;
        }
        pid = parent;
    }
    false
}

fn process_environ_is_pane_of_socket(pid: u32, socket: &str) -> bool {
    match read_process_environ(pid) {
        Some(bytes) => environ_is_pane_of_socket(&bytes, socket),
        None => false,
    }
}

#[cfg(target_os = "linux")]
fn read_process_environ(pid: u32) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ")).ok()
}

#[cfg(target_os = "macos")]
fn read_process_environ(pid: u32) -> Option<Vec<u8>> {
    // Same-user `ps` prints the environment on the command line. A short read
    // is treated as "not a pane" so a lookup failure cannot block a human
    // shell; the Linux path reads `/proc` directly.
    let output = std::process::Command::new("/bin/ps")
        .args(["eww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_process_environ(_pid: u32) -> Option<Vec<u8>> {
    None
}

/// `PLEXI_PANE_ID` is set and `PLEXI_SOCKET` is exactly `socket`.
pub(crate) fn environ_is_pane_of_socket(bytes: &[u8], socket: &str) -> bool {
    let mut pane_id = false;
    let mut found_socket = false;
    for entry in bytes.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        // macOS `ps` separates variables with spaces. Accept either form.
        let text = String::from_utf8_lossy(entry);
        for token in text.split(|ch: char| ch == '\0' || ch.is_whitespace()) {
            if let Some(value) = token.strip_prefix("PLEXI_PANE_ID=") {
                pane_id = !value.is_empty();
            } else if let Some(value) = token.strip_prefix("PLEXI_SOCKET=") {
                found_socket = sockets_match(value, socket);
            }
        }
    }
    pane_id && found_socket
}

fn sockets_match(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    Path::new(left) == Path::new(right)
}

#[cfg(test)]
mod tests {
    use super::environ_is_pane_of_socket;

    #[test]
    fn environ_matches_only_the_same_host_socket() {
        let bytes = b"HOME=/tmp\0PLEXI_PANE_ID=4\0PLEXI_SOCKET=/tmp/plexi.sock\0";
        assert!(environ_is_pane_of_socket(bytes, "/tmp/plexi.sock"));
        assert!(!environ_is_pane_of_socket(
            bytes,
            "/tmp/other-host.sock"
        ));
    }

    #[test]
    fn empty_pane_id_is_not_a_pane() {
        let bytes = b"PLEXI_PANE_ID=\0PLEXI_SOCKET=/tmp/plexi.sock\0";
        assert!(!environ_is_pane_of_socket(bytes, "/tmp/plexi.sock"));
    }

    #[test]
    fn missing_socket_is_not_a_pane_of_this_host() {
        let bytes = b"PLEXI_PANE_ID=4\0";
        assert!(!environ_is_pane_of_socket(bytes, "/tmp/plexi.sock"));
    }
}
