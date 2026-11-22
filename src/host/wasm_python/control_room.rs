//! Room in a Python guest's stdin held for directed-pipe control events
//! (stint 1681).
//!
//! `DirectedPipeIncoming`, `DirectedPipeOpened`, `DirectedPipeOpenRefused`
//! and `PipeClosed` change which connection a guest may use, so each one the
//! host promises must reach the guest even when ordinary traffic has filled
//! its stdin. The host reserves room for such an event before it commits to
//! sending it; ordinary pushes never use reserved room, and a reserved event
//! is enqueued into its own room. Reservation and enqueue both check that
//! stdin is open under its lock, and stdin closes when the guest thread
//! exits, so nothing is promised to a guest that has finished.
//!
//! Room comes from three pools inside the stdin cap. Answers are short-lived:
//! each admitted open holds one until the host answers it. Close room is held
//! for a connection's whole life, split between connections this guest opened
//! and connections opened to it, so neither long-lived kind can starve the
//! answers this guest needs to make progress, and a peer opening many
//! connections to it cannot exhaust the room for its own opens. An open is
//! admitted on its answer room alone when its opener's live connections
//! hold all the opener close room: a reopen of one of them needs none, and a
//! fresh open takes its close room at dispatch or is refused.

use std::sync::{Arc, Mutex, Weak};

use super::{AppendableStdin, InputLifecycle, InputState, WasmPythonError};

/// Which room a reservation comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlPool {
    /// An admitted open's one answer: `Opened` or `OpenRefused`.
    Answers,
    /// The `PipeClosed` of a connection this guest opened.
    OpenerCloses,
    /// The `PipeClosed` of a connection another view opened to this guest.
    TargetCloses,
}

impl ControlPool {
    /// The most room the pool may hold, as a share of the stdin cap.
    fn budget(self, cap: usize) -> usize {
        match self {
            Self::Answers => cap / 16,
            Self::OpenerCloses | Self::TargetCloses => cap / 8,
        }
    }
}

/// Bytes each pool holds.
#[derive(Default, Debug)]
pub(super) struct ControlHeld {
    answers: usize,
    opener_closes: usize,
    target_closes: usize,
}

impl ControlHeld {
    fn pool_mut(&mut self, pool: ControlPool) -> &mut usize {
        match pool {
            ControlPool::Answers => &mut self.answers,
            ControlPool::OpenerCloses => &mut self.opener_closes,
            ControlPool::TargetCloses => &mut self.target_closes,
        }
    }

    pub(super) fn total(&self) -> usize {
        self.answers + self.opener_closes + self.target_closes
    }
}

/// Why room could not be reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoRoom {
    /// Stdin is closed: the guest has finished. Nothing will ever fit.
    Closed,
    /// Not now: stdin is full until the guest reads.
    Full,
    /// Not now: the pool's room is all held until held room is released.
    PoolFull,
    /// The event is larger than its pool could ever hold.
    Oversize,
}

/// Room held in one stdin for one control event. Enqueued with
/// [`AppendableStdin::push_reserved`]; dropped unused, it is released.
#[must_use = "held room is released when dropped; enqueue into it or drop it on purpose"]
pub(crate) struct StdinSlot {
    state: Weak<Mutex<InputState>>,
    pool: ControlPool,
    bytes: usize,
}

impl std::fmt::Debug for StdinSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdinSlot")
            .field("pool", &self.pool)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl StdinSlot {
    /// Whether this room is held in `stdin`.
    fn held_in(&self, stdin: &AppendableStdin) -> bool {
        std::ptr::eq(self.state.as_ptr(), Arc::as_ptr(&stdin.state))
    }
}

impl Drop for StdinSlot {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let wake = {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            let held = state.control.pool_mut(self.pool);
            *held = held.saturating_sub(self.bytes);
            state.capacity_wake.take()
        };
        if let Some(wake) = wake {
            wake();
        }
    }
}

/// Wake fired once, outside the stdin lock, the next time room may have
/// freed: the guest read, held room was released or used, or stdin closed.
pub(crate) type CapacityWake = Arc<dyn Fn() + Send + Sync>;

impl AppendableStdin {
    /// Hold `bytes` of room in `pool` for one control event.
    pub(crate) fn reserve(&self, pool: ControlPool, bytes: usize) -> Result<StdinSlot, NoRoom> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.hold(&mut state, pool, bytes)
    }

    /// Admit an open request this guest made: hold room for its answer, and
    /// for the `PipeClosed` of the connection it may become when the opener
    /// close pool has room. An open admitted without close room is refused
    /// at dispatch unless it names a connection it already holds, which
    /// needs no new close room. When stdin or the answer pool has no room,
    /// `wake` is armed under the same lock that found it full, so a read or
    /// release that makes room always fires it.
    pub(crate) fn reserve_open(
        &self,
        answer_bytes: usize,
        close_bytes: usize,
        wake: &CapacityWake,
    ) -> Result<(StdinSlot, Option<StdinSlot>), NoRoom> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let answer = match self.hold(&mut state, ControlPool::Answers, answer_bytes) {
            Ok(answer) => answer,
            Err(error @ (NoRoom::Full | NoRoom::PoolFull)) => {
                state.capacity_wake = Some(Arc::clone(wake));
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let close = match self.hold(&mut state, ControlPool::OpenerCloses, close_bytes) {
            Ok(close) => Some(close),
            Err(NoRoom::PoolFull | NoRoom::Oversize) => None,
            Err(error) => {
                // Release the answer's room under this lock: dropping the
                // slot would take the wake this is about to arm.
                let held = state.control.pool_mut(ControlPool::Answers);
                *held = held.saturating_sub(answer.bytes);
                let mut answer = answer;
                answer.bytes = 0;
                if error == NoRoom::Full {
                    state.capacity_wake = Some(Arc::clone(wake));
                }
                return Err(error);
            }
        };
        Ok((answer, close))
    }

    fn hold(
        &self,
        state: &mut InputState,
        pool: ControlPool,
        bytes: usize,
    ) -> Result<StdinSlot, NoRoom> {
        if matches!(state.lifecycle, InputLifecycle::Closed) {
            return Err(NoRoom::Closed);
        }
        let budget = pool.budget(state.cap);
        if bytes > budget {
            return Err(NoRoom::Oversize);
        }
        if *state.control.pool_mut(pool) + bytes > budget {
            return Err(NoRoom::PoolFull);
        }
        if state.bytes.len() + state.control.total() + bytes > state.cap {
            return Err(NoRoom::Full);
        }
        *state.control.pool_mut(pool) += bytes;
        Ok(StdinSlot {
            state: Arc::downgrade(&self.state),
            pool,
            bytes,
        })
    }

    /// Enqueue one control event's line into the room `slot` holds in this
    /// stdin. Fails, releasing the room, when stdin has closed; a slot from
    /// another stdin or one too small for the line is a host bug and fails
    /// loudly without enqueueing.
    pub(crate) fn push_reserved(
        &self,
        mut slot: StdinSlot,
        line: &[u8],
    ) -> Result<(), WasmPythonError> {
        if !slot.held_in(self) {
            return Err(WasmPythonError::ControlRoom(
                "room held in another stdin".to_string(),
            ));
        }
        if line.len() > slot.bytes {
            return Err(WasmPythonError::ControlRoom(format!(
                "a {}-byte event in {} bytes of held room",
                line.len(),
                slot.bytes
            )));
        }
        let (waker, wake) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let InputState {
                bytes: queued,
                lifecycle,
                control,
                capacity_wake,
                ..
            } = &mut *state;
            let open = match lifecycle {
                InputLifecycle::Open(open) => open,
                InputLifecycle::Closed => return Err(WasmPythonError::StdinClosed),
            };
            let held = control.pool_mut(slot.pool);
            *held = held.saturating_sub(slot.bytes);
            slot.bytes = 0;
            queued.extend(line);
            // The pool has room again, though stdin as a whole does not.
            (open.waker.take(), capacity_wake.take())
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        if let Some(wake) = wake {
            wake();
        }
        Ok(())
    }

    /// Whether the guest can still read: stdin closes when its thread exits.
    pub(crate) fn is_open(&self) -> bool {
        matches!(
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .lifecycle,
            InputLifecycle::Open(_)
        )
    }

    /// Test hook: a stdin of `cap` bytes, so a test can fill it cheaply.
    #[cfg(test)]
    pub(crate) fn set_cap_for_test(&self, cap: usize) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).cap = cap;
    }

    /// Test hook: while held, the guest reads nothing, as if it were busy.
    #[cfg(test)]
    pub(crate) fn hold_reads_for_test(&self, held: bool) {
        let waker = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.reads_held = held;
            match &mut state.lifecycle {
                InputLifecycle::Open(open) if !held => open.waker.take(),
                _ => None,
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Test hook: fill stdin with one blank line the guest skips, leaving
    /// exactly the held room free. Returns the bytes pushed.
    #[cfg(test)]
    pub(crate) fn fill_for_test(&self) -> usize {
        let free = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.cap - state.bytes.len() - state.control.total()
        };
        if free == 0 {
            return 0;
        }
        let mut line = vec![b' '; free];
        line[free - 1] = b'\n';
        self.push(&line).expect("fill stdin");
        free
    }

    /// Test hook: `hook` runs after each push of a line that contains
    /// `needle`, outside the stdin lock.
    #[cfg(test)]
    pub(crate) fn on_push_for_test(&self, needle: &str, hook: Arc<dyn Fn() + Send + Sync>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_hook = Some((needle.as_bytes().to_vec(), hook));
    }

    /// Test hook: the bytes each pool holds, answers first.
    #[cfg(test)]
    pub(crate) fn held_for_test(&self) -> (usize, usize, usize) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (
            state.control.answers,
            state.control.opener_closes,
            state.control.target_closes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdin(cap: usize) -> AppendableStdin {
        let stdin = AppendableStdin::default();
        stdin.set_cap_for_test(cap);
        stdin
    }

    #[test]
    fn ordinary_pushes_never_use_held_room() {
        let stdin = stdin(1600);
        let slot = stdin.reserve(ControlPool::Answers, 100).expect("room");
        stdin
            .push(&[b'x'; 1500])
            .expect("ordinary fits beside the held room");
        assert!(
            stdin.push(&[b'x'; 1]).is_err(),
            "the held room is not ordinary room"
        );
        stdin
            .push_reserved(slot, &[b'y'; 80])
            .expect("the held event fits");
        assert_eq!(stdin.buffered_bytes(), 1580);
        assert_eq!(stdin.held_for_test(), (0, 0, 0));
    }

    #[test]
    fn each_pool_is_bounded_so_long_lived_closes_cannot_take_the_answers_room() {
        let stdin = stdin(1600);
        // Closes may hold an eighth of the cap per kind, answers a sixteenth.
        let opener_closes: Vec<_> = (0..2)
            .map(|_| stdin.reserve(ControlPool::OpenerCloses, 100).expect("room"))
            .collect();
        assert_eq!(
            stdin.reserve(ControlPool::OpenerCloses, 1).err(),
            Some(NoRoom::PoolFull)
        );
        let target_closes: Vec<_> = (0..2)
            .map(|_| stdin.reserve(ControlPool::TargetCloses, 100).expect("room"))
            .collect();
        assert_eq!(
            stdin.reserve(ControlPool::TargetCloses, 1).err(),
            Some(NoRoom::PoolFull)
        );
        let answer = stdin
            .reserve(ControlPool::Answers, 100)
            .expect("answers keep their own room");
        assert_eq!(
            stdin.reserve(ControlPool::Answers, 1).err(),
            Some(NoRoom::PoolFull)
        );
        assert_eq!(
            stdin.reserve(ControlPool::Answers, 101).err(),
            Some(NoRoom::Oversize),
            "an event larger than its pool never fits"
        );
        drop(answer);
        assert!(stdin.reserve(ControlPool::Answers, 100).is_ok());
        drop((opener_closes, target_closes));
        assert_eq!(stdin.held_for_test(), (0, 0, 0));
    }

    #[test]
    fn a_full_stdin_has_no_room_until_the_guest_reads() {
        let stdin = stdin(1600);
        stdin.fill_for_test();
        assert_eq!(
            stdin.reserve(ControlPool::Answers, 10).err(),
            Some(NoRoom::Full)
        );
        let freed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&freed);
        let wake: CapacityWake =
            Arc::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            stdin.reserve_open(10, 10, &wake).err(),
            Some(NoRoom::Full),
            "an open finding stdin full arms the wake as it fails"
        );
        assert!(!freed.load(std::sync::atomic::Ordering::SeqCst));
        let mut storage = vec![0u8; 64];
        let mut reader = std::pin::pin!(stdin.clone());
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut buf = tokio::io::ReadBuf::new(&mut storage);
        assert!(tokio::io::AsyncRead::poll_read(reader.as_mut(), &mut cx, &mut buf).is_ready());
        assert!(
            freed.load(std::sync::atomic::Ordering::SeqCst),
            "a read wakes the held intake"
        );
        assert!(stdin.reserve(ControlPool::Answers, 10).is_ok());
    }

    #[test]
    fn a_closed_stdin_admits_nothing_and_held_room_cannot_be_used() {
        let stdin = stdin(1600);
        let slot = stdin.reserve(ControlPool::TargetCloses, 50).expect("room");
        stdin.close();
        assert_eq!(
            stdin.reserve(ControlPool::Answers, 10).err(),
            Some(NoRoom::Closed)
        );
        assert!(matches!(
            stdin.push_reserved(slot, b"{}\n"),
            Err(WasmPythonError::StdinClosed)
        ));
        assert!(!stdin.is_open());
    }

    #[test]
    fn room_held_in_one_stdin_is_never_used_in_another() {
        let old = stdin(1600);
        let new = stdin(1600);
        let slot = old.reserve(ControlPool::Answers, 50).expect("room");
        assert!(matches!(
            new.push_reserved(slot, b"{}\n"),
            Err(WasmPythonError::ControlRoom(_))
        ));
        assert_eq!(new.buffered_bytes(), 0);
        assert_eq!(
            old.held_for_test(),
            (0, 0, 0),
            "the refused slot was released"
        );
    }

    fn wake_flag() -> (CapacityWake, Arc<std::sync::atomic::AtomicBool>) {
        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&fired);
        let wake: CapacityWake =
            Arc::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst));
        (wake, fired)
    }

    #[test]
    fn an_open_is_admitted_without_close_room_when_live_connections_hold_it_all() {
        let stdin = stdin(1600);
        let live: Vec<_> = (0..2)
            .map(|_| stdin.reserve(ControlPool::OpenerCloses, 100).expect("room"))
            .collect();
        let (wake, fired) = wake_flag();
        let (answer, close) = stdin
            .reserve_open(50, 100, &wake)
            .expect("the answer has room; the close pool being held does not stall the open");
        assert!(close.is_none(), "no close room is held for it");
        assert_eq!(stdin.held_for_test(), (50, 200, 0));
        drop(answer);
        assert!(
            !fired.load(std::sync::atomic::Ordering::SeqCst),
            "an admitted open arms nothing"
        );
        drop(live);
        let (_answer, close) = stdin.reserve_open(50, 100, &wake).expect("room");
        assert!(close.is_some(), "close room is held once the pool has it");
    }

    #[test]
    fn an_open_without_room_for_its_close_in_stdin_holds_nothing_and_arms_the_wake() {
        let stdin = stdin(1600);
        // Leave room for the answer, not for the close as well.
        stdin.push(&[b'x'; 1500]).expect("ordinary");
        let (wake, fired) = wake_flag();
        assert_eq!(stdin.reserve_open(50, 100, &wake).err(), Some(NoRoom::Full));
        assert_eq!(
            stdin.held_for_test(),
            (0, 0, 0),
            "the answer's room was released"
        );
        assert!(
            !fired.load(std::sync::atomic::Ordering::SeqCst),
            "releasing the answer's own room does not fire the wake it armed"
        );
        let mut storage = vec![0u8; 64];
        let mut reader = std::pin::pin!(stdin.clone());
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut buf = tokio::io::ReadBuf::new(&mut storage);
        assert!(tokio::io::AsyncRead::poll_read(reader.as_mut(), &mut cx, &mut buf).is_ready());
        assert!(
            fired.load(std::sync::atomic::Ordering::SeqCst),
            "a read fires it"
        );
    }
}
