//! Pane liveness: the host's own observation of a pane, kept structurally
//! separate from any claim the pane's occupant makes about itself.
//!
//! Stint 0665. A pane's `status` slot is a claim an agent writes about
//! itself (`state=running`, with a step token embedded in the value by
//! convention) — it is not liveness. A wedged head sat 3h10m with its own
//! status slot still reading `running` because the two facts shared one
//! field (`docs/agent-run-orchestration.md`, "A status slot that is a
//! claim, not liveness"). `compute` is the single accessor for both: it
//! always returns the claim and the observation together, in one object, so
//! a caller cannot read one without the other.

use crate::host::pane::Pane;

/// Read the pane's `status` slot (the agent-authored liveness claim), if one
/// has been written. The value is opaque to the host — an agent embeds its
/// own step token in it by convention (e.g. `"running:step-3"`); the host
/// never parses it, only reports it alongside its age.
fn claimed_state(pane: &Pane) -> Option<serde_json::Value> {
    let path = pane.slots()?.get("status")?;
    let value = std::fs::read_to_string(path).ok()?;
    let written_seconds_ago = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
        .map(|elapsed| elapsed.as_secs());
    Some(serde_json::json!({
        "value": value,
        "written_seconds_ago": written_seconds_ago,
    }))
}

/// True when `pid` names a live process. Best-effort: a `kill(pid, 0)` that
/// fails with `EPERM` (process exists, no permission) would misreport, but
/// every pid this module checks is a host-spawned child, so permission is
/// never the failure mode in practice.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Windows has no signal-0 probe. `OpenProcess` for the query-limited right
/// succeeds for a live process and for a zombie one whose handle is still
/// held, so the exit code is checked too: `STILL_ACTIVE` is the only answer
/// that counts as alive.
#[cfg(windows)]
fn pid_is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return false;
    }
    // SAFETY: plain scalar arguments; a null return is the documented failure
    // signal and is checked before any use.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut exit_code: u32 = 0;
    // SAFETY: `handle` is live until the CloseHandle below; `exit_code` is a
    // local out-param.
    let alive = unsafe {
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    };
    alive
}

/// Direct children of `pid`, via `pgrep -P` — never `proc_listchildpids`,
/// which returns `EFAULT` on macOS 23.x/Sonoma (root `AGENTS.md` Traps).
/// `pgrep` exits 1 with empty stdout when there are no children; that is
/// not a failure, just an empty inventory.
fn child_pids_via_pgrep(pid: u32) -> Vec<u32> {
    let output = std::process::Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output();
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect(),
        Err(error) => {
            log::warn!("pane_liveness: pgrep -P {pid} failed to spawn: {error}");
            Vec::new()
        }
    }
}

/// Host-derived liveness observation. Never trusts any pane-authored value.
fn observed_state(pane: &Pane) -> serde_json::Value {
    match pane {
        Pane::Terminal(terminal) => {
            let shell_pid = terminal.backend.child_pid();
            let process_alive = !terminal.exited && pid_is_alive(shell_pid);
            let last_output_seconds_ago =
                terminal.last_pty_output_at.map(|at| at.elapsed().as_secs());
            let child_pids = if process_alive {
                child_pids_via_pgrep(shell_pid)
            } else {
                Vec::new()
            };
            let foreground_command = terminal
                .backend
                .foreground_pid()
                .filter(|fg| *fg != shell_pid as i32)
                .and_then(|fg| crate::host::shell::get_pid_name(fg as u32));
            serde_json::json!({
                "process_alive": process_alive,
                "last_output_seconds_ago": last_output_seconds_ago,
                "child_pids": child_pids,
                "foreground_command": foreground_command,
            })
        }
        Pane::App(app_pane) => {
            let (lifecycle, _) = app_pane.runtime.lifecycle();
            let process_alive = !matches!(lifecycle, "exited" | "failed" | "crashed");
            // App runtimes (in-process WASM/Python) have no OS child of their
            // own to inventory and no PTY output stream to time-stamp; a
            // future runtime with a real subprocess can extend this arm
            // without changing the shape callers see.
            serde_json::json!({
                "process_alive": process_alive,
                "last_output_seconds_ago": Option::<u64>::None,
                "child_pids": Vec::<u32>::new(),
                "foreground_command": Option::<String>::None,
            })
        }
        Pane::Portal(_) => serde_json::json!({
            "process_alive": false,
            "last_output_seconds_ago": Option::<u64>::None,
            "child_pids": Vec::<u32>::new(),
            "foreground_command": Option::<String>::None,
        }),
    }
}

/// Raise a typed `stale-claim` condition only when the caller supplied a
/// window (`--stale-after <s>` / a request field): no window means no
/// evaluation, never an invented default (per stint 0665's Ruling). Also
/// requires a claim to exist — an absent claim is a different condition,
/// not staleness — and requires the host to have an output timestamp to
/// compare against.
fn evaluate_stale_claim(
    claimed: &Option<serde_json::Value>,
    observed: &serde_json::Value,
    stale_after_secs: Option<u64>,
) -> Option<serde_json::Value> {
    let stale_after_secs = stale_after_secs?;
    claimed.as_ref()?;
    let idle_seconds = observed.get("last_output_seconds_ago")?.as_u64()?;
    if idle_seconds < stale_after_secs {
        return None;
    }
    log::info!(
        "pane_liveness: stale-claim raised: idle_seconds={idle_seconds} stale_after_seconds={stale_after_secs}"
    );
    Some(serde_json::json!({
        "idle_seconds": idle_seconds,
        "stale_after_seconds": stale_after_secs,
    }))
}

/// The single accessor for a pane's liveness: the agent's own claim and the
/// host's observation, always together. `stale_after_secs` comes from
/// `plexi pane state --stale-after <secs>`; omit it (`None`) to skip
/// stale-claim evaluation entirely.
pub fn compute(pane: &Pane, stale_after_secs: Option<u64>) -> serde_json::Value {
    let claimed = claimed_state(pane);
    let observed = observed_state(pane);
    let stale = evaluate_stale_claim(&claimed, &observed, stale_after_secs);
    log::info!(
        "pane_liveness: computed: claimed_present={} process_alive={} last_output_seconds_ago={} stale_claim={}",
        claimed.is_some(),
        observed["process_alive"],
        observed["last_output_seconds_ago"],
        stale.is_some(),
    );
    serde_json::json!({
        "claimed_state": claimed,
        "observed_state": observed,
        "stale_claim": stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_claim_absent_without_a_window() {
        let claimed = Some(serde_json::json!({"value": "running", "written_seconds_ago": 1}));
        let observed = serde_json::json!({"last_output_seconds_ago": 10_000});
        assert!(evaluate_stale_claim(&claimed, &observed, None).is_none());
    }

    #[test]
    fn stale_claim_absent_without_a_claim() {
        let observed = serde_json::json!({"last_output_seconds_ago": 10_000});
        assert!(evaluate_stale_claim(&None, &observed, Some(60)).is_none());
    }

    #[test]
    fn stale_claim_absent_when_output_is_recent() {
        let claimed = Some(serde_json::json!({"value": "running", "written_seconds_ago": 1}));
        let observed = serde_json::json!({"last_output_seconds_ago": 5});
        assert!(evaluate_stale_claim(&claimed, &observed, Some(60)).is_none());
    }

    #[test]
    fn stale_claim_raised_when_output_exceeds_the_window() {
        let claimed = Some(serde_json::json!({"value": "running", "written_seconds_ago": 1}));
        let observed = serde_json::json!({"last_output_seconds_ago": 11_400});
        let stale = evaluate_stale_claim(&claimed, &observed, Some(60)).expect("stale-claim");
        assert_eq!(stale["idle_seconds"], 11_400);
        assert_eq!(stale["stale_after_seconds"], 60);
    }
}
