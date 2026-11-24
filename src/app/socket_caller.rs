//! The one classification of a command-socket peer (stint 1285). It runs on
//! the host thread, once per request, before any handler or stream service.

use crate::broker::caller::{
    PaneCaller, PeerWitness, SocketCaller, UnidentifiedReason,
};

use super::PlexiApp;

impl PlexiApp {
    /// Who `peer` is. The nearest recorded process that is exactly one
    /// pane's terminal shell, by pid and kernel start time, names a pane
    /// caller (a terminal covered by a view included). With no match, only a
    /// walk that reached the OS root names an external caller. Everything
    /// else is unidentified.
    pub(crate) fn classify_socket_peer(&self, peer: Option<&PeerWitness>) -> SocketCaller {
        self.classify_socket_peer_excluding(peer, None)
    }

    /// [`Self::classify_socket_peer`] as it will be once pane `excluded` is
    /// gone: the classification a request launches under when its own work
    /// first closes that pane (stint 1285, Codex 3290 r4).
    pub(crate) fn classify_socket_peer_excluding(
        &self,
        peer: Option<&PeerWitness>,
        excluded: Option<crate::spatial::tiling::PaneId>,
    ) -> SocketCaller {
        let Some(peer) = peer else {
            return SocketCaller::Unidentified(UnidentifiedReason::NoPeerCredential);
        };
        // No process facts at all: the peer itself could not be read.
        if peer.hops.is_empty() {
            return SocketCaller::Unidentified(UnidentifiedReason::LineageIncomplete(peer.end));
        }
        for hop in &peer.hops {
            let mut matches = self.windows.iter().flat_map(|window| {
                window.panes.iter().filter_map(move |(visible_pane, pane)| {
                    if excluded == Some(*visible_pane) {
                        return None;
                    }
                    pane.terminal_incarnation()
                        .filter(|terminal| terminal.is_shell_process(hop.pid, hop.start))
                        .map(|terminal| (window, *visible_pane, terminal))
                })
            });
            let Some((window, visible_pane, terminal)) = matches.next() else {
                continue;
            };
            if matches.next().is_some() {
                return SocketCaller::Unidentified(UnidentifiedReason::Ambiguous);
            }
            let Some(authority) = terminal.authority().cloned() else {
                return SocketCaller::Unidentified(UnidentifiedReason::PaneWithoutAuthority);
            };
            return SocketCaller::Pane(PaneCaller {
                terminal_pane: terminal.id,
                visible_pane,
                context_id: window.context_id,
                window_id: window.window_id,
                peer: peer.clone(),
                authority,
            });
        }
        match peer.end {
            crate::broker::caller::AncestryEnd::ReachedRoot => {
                SocketCaller::External(peer.clone())
            }
            end => SocketCaller::Unidentified(UnidentifiedReason::LineageIncomplete(end)),
        }
    }
}

#[cfg(test)]
impl PlexiApp {
    /// The origin a socket request from these live processes would get.
    pub(crate) fn socket_origin_for_pids(
        &self,
        pids: &[u32],
    ) -> crate::broker::caller::RequestOrigin {
        crate::broker::caller::RequestOrigin::Socket(
            self.classify_socket_peer(Some(&PeerWitness::for_live_pids(pids))),
        )
    }
}

/// How a pane-IPC message entered the host's queue.
#[derive(Debug)]
pub(crate) enum Ingress {
    /// The command socket: the kernel witness captured at accept (absent
    /// when the kernel gave none) and the request's reply token.
    Socket {
        peer: Option<PeerWitness>,
        reply: Option<String>,
    },
    /// A host thread (stream services waking the host).
    Host,
}

/// One message on the pane-IPC queue. The host thread classifies a socket
/// ingress once, at drain, before any handler or stream service runs.
#[derive(Debug)]
pub(crate) enum PaneIpcMessage {
    /// A `HostRequest` and where it came from.
    Request {
        ingress: Ingress,
        request: Box<crate::protocol::HostRequest>,
    },
    /// A streaming frame the socket thread serves itself, waiting for the
    /// host thread's admission of its peer.
    Admit {
        peer: Option<PeerWitness>,
        frame: super::socket_policy::StreamFrame,
        /// The workspace a `command_view_follow` names, checked against the
        /// caller's scope on the host thread.
        scope: Option<String>,
        reply: std::sync::mpsc::SyncSender<
            Result<StreamAdmission, super::socket_policy::Refusal>,
        >,
    },
}

/// What the host thread admitted a streaming frame to: the classified caller,
/// and for a dashboard follow the permit every emission commits against.
#[derive(Debug)]
pub(crate) struct StreamAdmission {
    pub(crate) caller: SocketCaller,
    pub(crate) dashboard: Option<crate::broker::permit::Permit<crate::broker::permit::ReadDashboard>>,
}

impl PaneIpcMessage {
    pub(crate) fn host(request: crate::protocol::HostRequest) -> Self {
        Self::Request {
            ingress: Ingress::Host,
            request: Box::new(request),
        }
    }
}
