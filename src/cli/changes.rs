//! `plexi changes` — preview, accept, refresh, and revert a prepared edit.
//!
//! Propose and accept both admit through the permission monitor before any
//! disk write. Without a grant the command prints `permission_required` and
//! leaves the file alone.

use std::path::Path;

use crate::host::changes::{self, GateStop};

fn stop(error: GateStop) -> i32 {
    match error {
        GateStop::Required { pending_request_id } => {
            println!("permission_required");
            println!("pending_request_id={pending_request_id}");
            2
        }
        GateStop::Failed(message) if message.starts_with("stale:") => {
            eprintln!("error: {message}");
            println!("status=stale");
            3
        }
        GateStop::Failed(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

pub fn changes_allow_cli(agent: &str, file: &Path, old: &str, new: &str) -> i32 {
    match changes::allow_edit(agent, file, old, new) {
        Ok(()) => {
            println!("allowed");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

pub fn changes_propose_cli(agent: &str, file: &Path, old: &str, new: &str) -> i32 {
    match changes::propose_gated(agent, file, old, new) {
        Ok(prepared) => {
            println!("change_set={}", prepared.id);
            println!("status=pending");
            println!("applied=false");
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_preview_cli(id: &str) -> i32 {
    match changes::preview(id) {
        Ok(diff) => {
            print!("{diff}");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

pub fn changes_accept_cli(id: &str) -> i32 {
    match changes::accept_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status=committed");
            println!("agent={}", set.agent_id);
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_refresh_cli(id: &str) -> i32 {
    match changes::refresh_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status={}", set.status);
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_revert_cli(id: &str) -> i32 {
    match changes::revert_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status=reverted");
            println!("agent={}", set.agent_id);
            0
        }
        Err(error) => stop(error),
    }
}
