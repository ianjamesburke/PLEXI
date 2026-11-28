//! Who sent a request: the one typed caller every handler receives
//! (stint 1285, `.stint/runs/design-1285/NOTE-2.md`).
//!
//! The command socket learns its peer from the kernel at accept time
//! ([`PeerWitness`]). The host thread classifies that witness once per
//! request into a [`SocketCaller`]. Nothing on the wire names a caller, and
//! no socket caller is the person at the desktop: a decision the person
//! makes is proved only by [`crate::broker::consent::DecisionEvidence`] minted by the consent
//! surface that displayed it.

use std::sync::Arc;

use crate::broker::pane_authority::LivePaneAuthority;
use crate::broker::principal::{PluginInstance, Principal};
use crate::platform::process::{ProcInfo, ProcStart};
use plexi_host_model::model::PaneId;

/// How the host's walk up a socket peer's process ancestry ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AncestryEnd {
    /// The walk reached the OS root process: the lineage is complete.
    ReachedRoot,
    /// A process lookup failed before the root.
    LookupFailed,
    /// A process repeated.
    Cycle,
    /// The walk stopped at its hop limit.
    HopLimit,
}

/// One process in a peer's lineage, fixed to its kernel incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerHop {
    pub pid: u32,
    pub start: ProcStart,
}

impl PeerHop {
    /// Whether this exact process is still running: the pid is alive at
    /// the same kernel start time, so a reused pid is not it.
    pub fn is_live(&self) -> bool {
        crate::platform::process::process_info(self.pid).is_some_and(|info| info.start == self.start)
    }
}

/// The kernel's account of a socket peer, captured synchronously when the
/// connection is accepted: the connecting pid, then each process up its
/// lineage (nearest first) with its start time, and how the walk ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerWitness {
    /// The connecting process, as the kernel credential named it.
    pub pid: u32,
    pub hops: Vec<PeerHop>,
    pub end: AncestryEnd,
}

/// Bound on the walk so a lookup failure or a recycled pid mid-walk cannot
/// spin forever. The bound is not a control: a peer that is cut off from
/// its pane by it is Unidentified, never more.
pub const MAX_ANCESTOR_HOPS: usize = 32;

impl PeerWitness {
    /// Walk `peer_pid`'s lineage with the kernel.
    pub fn capture(peer_pid: u32) -> Self {
        Self::walk(peer_pid, crate::platform::process::process_info)
    }

    /// The walk over an injected process lookup. Each hop is recorded only
    /// with its start time; a process whose facts cannot be read ends the
    /// walk unrecorded. Only reaching pid 0 or 1 proves the lineage complete.
    pub fn walk(peer_pid: u32, info_of: impl Fn(u32) -> Option<ProcInfo>) -> Self {
        let mut hops = Vec::new();
        let mut candidate = peer_pid;
        let mut seen = std::collections::HashSet::new();
        let end = loop {
            if candidate == 0 || candidate == 1 {
                break AncestryEnd::ReachedRoot;
            }
            if hops.len() == MAX_ANCESTOR_HOPS {
                break AncestryEnd::HopLimit;
            }
            if !seen.insert(candidate) {
                break AncestryEnd::Cycle;
            }
            let Some(info) = info_of(candidate) else {
                break AncestryEnd::LookupFailed;
            };
            hops.push(PeerHop {
                pid: candidate,
                start: info.start,
            });
            candidate = info.ppid;
        };
        Self {
            pid: peer_pid,
            hops,
            end,
        }
    }

    /// A lineage taken as complete, as a peer running under these processes
    /// would present it: a live pid carries its real kernel start, a pid
    /// that is not running a start no live process has (so it matches no
    /// pane, like a reused pid).
    #[cfg(test)]
    pub fn for_live_pids(pids: &[u32]) -> Self {
        let hops = pids
            .iter()
            .map(|&pid| PeerHop {
                pid,
                start: crate::platform::process::process_info(pid)
                    .map_or(ProcStart::for_test(0), |info| info.start),
            })
            .collect();
        Self {
            pid: pids.first().copied().unwrap_or_default(),
            hops,
            end: AncestryEnd::ReachedRoot,
        }
    }

    /// Whether `pid` is the connecting process or one of its recorded
    /// ancestors.
    pub fn has_in_lineage(&self, pid: u32) -> bool {
        self.hops.iter().any(|hop| hop.pid == pid)
    }
}

/// Why a socket peer could not be identified. Every refusal names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnidentifiedReason {
    /// The kernel gave no peer credential, or the request did not arrive
    /// on the command socket at all.
    NoPeerCredential,
    /// No recorded process matched a pane and the walk did not reach the
    /// OS root.
    LineageIncomplete(AncestryEnd),
    /// The nearest matching process matched more than one pane.
    Ambiguous,
    /// The matched terminal holds no authority binding (a test fixture;
    /// every production terminal has one).
    PaneWithoutAuthority,
}

impl std::fmt::Display for UnidentifiedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPeerCredential => f.write_str("the kernel gave no peer credential"),
            Self::LineageIncomplete(end) => {
                write!(f, "the peer's process lineage is incomplete ({end:?})")
            }
            Self::Ambiguous => f.write_str("the peer's process matches more than one pane"),
            Self::PaneWithoutAuthority => f.write_str("the peer's pane holds no authority"),
        }
    }
}

/// A socket caller running in a pane's terminal.
#[derive(Debug, Clone)]
pub struct PaneCaller {
    /// The terminal pane whose shell is the peer's nearest ancestor. Its
    /// grant and principal are the caller's.
    pub terminal_pane: PaneId,
    /// The pane occupying that terminal's tile (a covering view, or the
    /// terminal itself). Placement and navigation use it.
    pub visible_pane: PaneId,
    pub context_id: u64,
    pub window_id: u64,
    pub peer: PeerWitness,
    /// The terminal incarnation's authority handle. A sealed grant stays
    /// sealed; a deferred action re-checks this exact handle.
    pub authority: Arc<LivePaneAuthority>,
}

/// Two pane callers are the same when they name the same panes and hold
/// the same authority handle, not merely an equal grant.
impl PartialEq for PaneCaller {
    fn eq(&self, other: &Self) -> bool {
        self.terminal_pane == other.terminal_pane
            && self.visible_pane == other.visible_pane
            && self.context_id == other.context_id
            && self.window_id == other.window_id
            && self.peer == other.peer
            && Arc::ptr_eq(&self.authority, &other.authority)
    }
}

impl Eq for PaneCaller {}

impl PaneCaller {
    /// The principal grants are stored under: the key the pane's call
    /// credential is issued as.
    pub fn principal(&self) -> Principal {
        Principal::Pane(self.authority.principal())
    }
}

/// Who is on the other end of a command-socket request. Three outcomes and
/// no human one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketCaller {
    Pane(PaneCaller),
    /// A kernel-identified process whose complete lineage reaches the OS
    /// root without passing through any pane's terminal.
    External(PeerWitness),
    /// Not enough evidence to say. No principal, no grant, no placement.
    Unidentified(UnidentifiedReason),
}

impl SocketCaller {
    /// The class name every policy trace uses.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Pane(_) => "pane",
            Self::External(_) => "external",
            Self::Unidentified(_) => "unidentified",
        }
    }

    /// The stored principal this caller acts as; `None` for an unidentified
    /// caller, which acts as nobody.
    pub fn principal(&self) -> Option<Principal> {
        match self {
            Self::Pane(pane) => (!pane.authority.is_revoked()).then(|| pane.principal()),
            Self::External(_) => Some(Principal::External),
            Self::Unidentified(_) => None,
        }
    }

    /// The kernel witness, when there was one.
    pub fn peer(&self) -> Option<&PeerWitness> {
        match self {
            Self::Pane(pane) => Some(&pane.peer),
            Self::External(peer) => Some(peer),
            Self::Unidentified(_) => None,
        }
    }

    pub fn pane(&self) -> Option<&PaneCaller> {
        match self {
            Self::Pane(pane) => Some(pane),
            Self::External(_) | Self::Unidentified(_) => None,
        }
    }
}

/// Where a request entered the host. There is no human origin: a person's
/// gesture is attribution (`HostUi`), and a decision needs
/// [`crate::broker::consent::DecisionEvidence`] at the sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOrigin {
    /// The command socket.
    Socket(SocketCaller),
    /// A file in the profile's spawn queue: any same-user process.
    SpawnQueue,
    /// A plugin guest acting through the host.
    PluginGuest(PluginInstance),
    /// Host-owned input: palette, menus, builtin views, desktop gestures.
    HostUi,
    /// The host itself: boot, restore, routines, approved continuations.
    HostSystem,
}

impl RequestOrigin {
    /// The origin name every policy trace uses.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Socket(caller) => caller.class(),
            Self::SpawnQueue => "spawn_queue",
            Self::PluginGuest(_) => "plugin_guest",
            Self::HostUi => "host_ui",
            Self::HostSystem => "host_system",
        }
    }

    pub fn socket_caller(&self) -> Option<&SocketCaller> {
        match self {
            Self::Socket(caller) => Some(caller),
            _ => None,
        }
    }
    /// A process outside every pane: this test process, walked to the root.
    #[cfg(test)]
    pub fn outside_for_test() -> Self {
        Self::Socket(SocketCaller::External(PeerWitness::capture(std::process::id())))
    }

    /// A socket request whose peer the kernel did not name.
    #[cfg(test)]
    pub fn unidentified_for_test() -> Self {
        Self::Socket(SocketCaller::Unidentified(UnidentifiedReason::NoPeerCredential))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(pairs: &'static [(u32, u32)]) -> impl Fn(u32) -> Option<ProcInfo> {
        move |pid| {
            pairs
                .iter()
                .find(|(child, _)| *child == pid)
                .map(|(_, ppid)| ProcInfo {
                    ppid: *ppid,
                    start: ProcStart::for_test(u64::from(pid)),
                })
        }
    }

    fn pids(walk: &PeerWitness) -> Vec<u32> {
        walk.hops.iter().map(|hop| hop.pid).collect()
    }

    #[test]
    fn the_walk_records_each_hop_with_its_start_and_how_it_ended() {
        let walk = PeerWitness::walk(30, chain(&[(30, 20), (20, 1)]));
        assert_eq!((pids(&walk), walk.end), (vec![30, 20], AncestryEnd::ReachedRoot));
        assert_eq!(walk.hops[1].start, ProcStart::for_test(20));
        let walk = PeerWitness::walk(30, chain(&[(30, 20)]));
        assert_eq!((pids(&walk), walk.end), (vec![30], AncestryEnd::LookupFailed));
        let walk = PeerWitness::walk(30, chain(&[(30, 20), (20, 30)]));
        assert_eq!((pids(&walk), walk.end), (vec![30, 20], AncestryEnd::Cycle));
        let walk = PeerWitness::walk(1_000, |pid| {
            Some(ProcInfo {
                ppid: pid + 1,
                start: ProcStart::for_test(0),
            })
        });
        assert_eq!((walk.hops.len(), walk.end), (MAX_ANCESTOR_HOPS, AncestryEnd::HopLimit));
    }

    #[test]
    fn an_unreadable_peer_records_no_hop() {
        let walk = PeerWitness::walk(30, |_| None);
        assert_eq!((walk.pid, walk.hops.len(), walk.end), (30, 0, AncestryEnd::LookupFailed));
    }
}
