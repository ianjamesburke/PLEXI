//! Directed pipe connections (stint 1681).
//!
//! A directed pipe joins two view instances (pane slot plus incarnation),
//! never two pane slots, so a slot whose occupant changes never inherits one.
//! Every accepted open is a connection with a host-issued generation, and a
//! send names the generation it was written for when the guest emits it. A
//! send on a retired generation is dropped, even if the same two instances
//! later reopen the same name: a reopen always mints a new generation.
//!
//! `(context_id, pipe_id)` stays an exclusive index: one live connection per
//! name per context, so two contexts may each use a name.

use std::collections::HashMap;

use crate::app::view_trait::ViewEmitter;

/// One live connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectedPipe {
    pub(crate) context_id: u64,
    pub(crate) pipe_id: String,
    /// The opener, then the target.
    pub(crate) ends: (ViewEmitter, ViewEmitter),
}

impl DirectedPipe {
    pub(crate) fn has_end(&self, end: ViewEmitter) -> bool {
        self.ends.0 == end || self.ends.1 == end
    }

    /// The other end from `end`, or `None` when `end` is not an end.
    pub(crate) fn peer_of(&self, end: ViewEmitter) -> Option<ViewEmitter> {
        let (a, b) = self.ends;
        if end == a {
            Some(b)
        } else if end == b {
            Some(a)
        } else {
            None
        }
    }
}

/// Whether an open may proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenCheck {
    /// The name is free in the context.
    Free,
    /// The same two instances already hold the name, in either direction;
    /// the open is answered with this generation.
    AlreadyOpen(u64),
    /// Other instances hold the name.
    InUse,
}

#[derive(Default)]
pub(crate) struct DirectedPipes {
    connections: HashMap<u64, DirectedPipe>,
    by_name: HashMap<(u64, String), u64>,
    last_generation: u64,
}

impl DirectedPipes {
    pub(crate) fn check_open(
        &self,
        context_id: u64,
        pipe_id: &str,
        opener: ViewEmitter,
        target: ViewEmitter,
    ) -> OpenCheck {
        let Some(&generation) = self.by_name.get(&(context_id, pipe_id.to_string())) else {
            return OpenCheck::Free;
        };
        let (a, b) = self.connections[&generation].ends;
        if (a, b) == (opener, target) || (a, b) == (target, opener) {
            OpenCheck::AlreadyOpen(generation)
        } else {
            OpenCheck::InUse
        }
    }

    /// Record a connection whose name `check_open` found free, returning its
    /// new generation.
    pub(crate) fn commit(
        &mut self,
        context_id: u64,
        pipe_id: &str,
        opener: ViewEmitter,
        target: ViewEmitter,
    ) -> u64 {
        let key = (context_id, pipe_id.to_string());
        assert!(
            !self.by_name.contains_key(&key),
            "directed pipe '{pipe_id}' committed over a live connection in context {context_id}"
        );
        self.last_generation += 1;
        let generation = self.last_generation;
        self.by_name.insert(key, generation);
        self.connections.insert(
            generation,
            DirectedPipe {
                context_id,
                pipe_id: pipe_id.to_string(),
                ends: (opener, target),
            },
        );
        generation
    }

    pub(crate) fn get(&self, generation: u64) -> Option<&DirectedPipe> {
        self.connections.get(&generation)
    }

    /// The live generation holding `pipe_id` in `context_id`.
    #[cfg(test)]
    pub(crate) fn generation_of(&self, context_id: u64, pipe_id: &str) -> Option<u64> {
        self.by_name
            .get(&(context_id, pipe_id.to_string()))
            .copied()
    }

    /// Remove one connection, freeing its name.
    pub(crate) fn revoke(&mut self, generation: u64) -> Option<DirectedPipe> {
        let pipe = self.connections.remove(&generation)?;
        self.by_name
            .remove(&(pipe.context_id, pipe.pipe_id.clone()));
        Some(pipe)
    }

    /// Remove every connection `retire` selects, oldest first, returning
    /// them. No later send or open can reach a removed generation.
    pub(crate) fn retire_where(
        &mut self,
        retire: impl Fn(&DirectedPipe) -> bool,
    ) -> Vec<(u64, DirectedPipe)> {
        let mut generations: Vec<u64> = self
            .connections
            .iter()
            .filter(|(_, pipe)| retire(pipe))
            .map(|(&generation, _)| generation)
            .collect();
        generations.sort_unstable();
        generations
            .into_iter()
            .filter_map(|generation| Some((generation, self.revoke(generation)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(pane_id: u64) -> ViewEmitter {
        ViewEmitter {
            pane_id,
            incarnation: Default::default(),
        }
    }

    #[test]
    fn reopening_the_same_pair_in_either_direction_answers_the_live_generation() {
        let (a, b) = (end(1), end(2));
        let mut pipes = DirectedPipes::default();
        assert_eq!(pipes.check_open(7, "p", a, b), OpenCheck::Free);
        let generation = pipes.commit(7, "p", a, b);
        assert_eq!(
            pipes.check_open(7, "p", a, b),
            OpenCheck::AlreadyOpen(generation)
        );
        assert_eq!(
            pipes.check_open(7, "p", b, a),
            OpenCheck::AlreadyOpen(generation)
        );
        assert_eq!(pipes.check_open(7, "p", a, end(3)), OpenCheck::InUse);
        assert_eq!(
            pipes.check_open(8, "p", a, end(3)),
            OpenCheck::Free,
            "names are per context"
        );
    }

    #[test]
    fn the_same_instances_reopening_after_retirement_get_a_fresh_generation() {
        let (a, b) = (end(1), end(2));
        let mut pipes = DirectedPipes::default();
        let first = pipes.commit(7, "p", a, b);
        let retired = pipes.retire_where(|p| p.has_end(b));
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].0, first);
        assert_eq!(pipes.get(first), None, "a retired generation stays dead");
        assert_eq!(pipes.check_open(7, "p", a, b), OpenCheck::Free);
        let second = pipes.commit(7, "p", a, b);
        assert_ne!(second, first);
        assert_eq!(pipes.get(first), None);
        assert_eq!(pipes.generation_of(7, "p"), Some(second));
    }

    #[test]
    fn only_an_end_has_a_peer_and_retirement_happens_once() {
        let (a, b, stranger) = (end(1), end(2), end(3));
        let mut pipes = DirectedPipes::default();
        let generation = pipes.commit(7, "p", a, b);
        let pipe = pipes.get(generation).expect("live");
        assert_eq!(pipe.peer_of(a), Some(b));
        assert_eq!(pipe.peer_of(b), Some(a));
        assert_eq!(pipe.peer_of(stranger), None);
        assert!(pipes.retire_where(|p| p.has_end(stranger)).is_empty());
        assert_eq!(pipes.retire_where(|p| p.has_end(a)).len(), 1);
        assert!(
            pipes.retire_where(|p| p.has_end(b)).is_empty(),
            "the other end finds nothing left"
        );
        assert_eq!(pipes.get(generation), None);
    }

    #[test]
    fn a_replacement_instance_in_the_same_slot_is_not_an_end() {
        let (old, peer) = (end(1), end(2));
        let replacement = end(1);
        assert_ne!(old, replacement, "same slot, new incarnation");
        let mut pipes = DirectedPipes::default();
        let generation = pipes.commit(7, "p", old, peer);
        assert_eq!(pipes.get(generation).unwrap().peer_of(replacement), None);
    }
}
