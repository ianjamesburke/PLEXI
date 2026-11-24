//! Kernel facts about a process: its parent and its start time (stint 1285).
//!
//! A pid alone names a process only while it lives; the kernel reuses it.
//! The start time read with the same call fixes which incarnation of that pid
//! a fact belongs to, so a pid matched later can be checked against the
//! process the host actually spawned.

/// When a process started, as the kernel reports it, in microseconds since
/// the Unix epoch (macOS) or clock ticks since boot (Linux). Only equality
/// between two readings on one machine is meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcStart(u64);

impl ProcStart {
    /// A start time for a synthetic lineage in tests.
    #[cfg(test)]
    pub fn for_test(raw: u64) -> Self {
        Self(raw)
    }
}

/// A live process's parent and start time, read in one kernel call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcInfo {
    pub ppid: u32,
    pub start: ProcStart,
}

/// `pid`'s parent and start time, or `None` when the process is gone or the
/// platform has no supported lookup.
#[cfg(target_os = "macos")]
pub fn process_info(pid: u32) -> Option<ProcInfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a correctly sized, writable proc_bsdinfo buffer.
    let rc = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if rc != size {
        log::warn!("process_info: proc_pidinfo failed for pid {pid} (rc={rc})");
        return None;
    }
    Some(ProcInfo {
        ppid: info.pbi_ppid,
        start: ProcStart(
            info.pbi_start_tvsec
                .saturating_mul(1_000_000)
                .saturating_add(info.pbi_start_tvusec),
        ),
    })
}

/// `/proc/<pid>/stat`: the comm field is parenthesized and may hold spaces
/// or parens, so fields are counted after the last `)`. ppid is the second
/// field there and starttime the twentieth.
#[cfg(target_os = "linux")]
pub fn process_info(pid: u32) -> Option<ProcInfo> {
    let contents = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(contents) => contents,
        Err(error) => {
            log::warn!("process_info: read /proc/{pid}/stat failed: {error}");
            return None;
        }
    };
    let rest = contents.get(contents.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Some(ProcInfo {
        ppid: fields.get(1)?.parse().ok()?,
        start: ProcStart(fields.get(19)?.parse().ok()?),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_info(pid: u32) -> Option<ProcInfo> {
    let _ = pid;
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn own_process_has_a_stable_start_and_its_real_parent() {
        let me = std::process::id();
        let first = process_info(me).expect("own process is readable");
        let second = process_info(me).expect("own process is readable");
        assert_eq!(first.start, second.start);
        assert_eq!(first.ppid, std::os::unix::process::parent_id());
    }

    #[test]
    fn a_new_process_on_a_reused_pid_reads_a_different_start() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .expect("spawn sleep");
        let child_start = process_info(child.id()).expect("child readable").start;
        let own_start = process_info(std::process::id()).expect("own").start;
        assert_ne!(child_start, own_start);
        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");
        assert!(process_info(child.id()).is_none(), "a reaped pid reads nothing");
    }
}
