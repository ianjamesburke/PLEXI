//! Host-arbitrated, process-owned resource admission.
use super::args::LeaseCmd;
use serde_json::json;

pub(crate) fn dispatch(cmd: LeaseCmd) -> i32 {
    let (op, name, max, pid) = match cmd {
        LeaseCmd::Acquire { name, max, pid } => ("acquire", Some(name), Some(max), pid),
        LeaseCmd::Release { name, pid } => ("release", name, None, pid),
    };
    let holder_pid = match pid.or_else(parent_pid) {
        Some(pid) => pid,
        None => {
            eprintln!("lease: cannot identify parent process; supply --pid");
            return 1;
        }
    };
    match super::request(
        json!({"type": "resource_lease", "op": op, "name": name,
        "max": max, "holder_pid": holder_pid}),
        "lease",
        "lease",
    ) {
        Ok(content) => {
            println!("{content}");
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(reply) if reply["status"] == "paused" => 2,
                Ok(reply) if reply.get("error").is_none() => 0,
                _ => 1,
            }
        }
        Err(code) => code,
    }
}

fn parent_pid() -> Option<u32> {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    system.process(pid)?.parent().map(|pid| pid.as_u32())
}
