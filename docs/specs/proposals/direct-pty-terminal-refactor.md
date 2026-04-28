# Direct PTY Terminal Refactor

## Purpose

This document describes the fix required to make Plexi's terminal behavior match a real terminal emulator for interactive shells, `Ctrl+C`, `zoxide zi`, `fzf`, and other full-screen or terminal-query-driven tools.

This is a **future enhancement** document, not MVP scope by default.

## Problem Summary

Plexi's terminal sessions have broken signal handling and TUI support. `Ctrl+C` does not interrupt processes, `fzf` and `zoxide zi` hang, and the shell reports "no job control."

Current flow:

1. User types into `xterm.js`.
2. Frontend sends bytes to the backend RPC bridge.
3. Backend writes those bytes into `Bun.Terminal`.
4. Shell output comes back from `Bun.Terminal`.
5. Frontend writes that output into `xterm.js`.

## Root Cause (Corrected 2026-03-13)

The original version of this document diagnosed the problem as "double terminal emulation" — claiming `Bun.Terminal` was a backend terminal emulator competing with `xterm.js`. **This was incorrect.**

`Bun.Terminal` is a raw PTY wrapper, not a terminal emulator. It does not maintain a screen buffer, parse escape sequences, or render anything. xterm.js is the only emulator in the chain.

The actual root cause is a **bug in `Bun.Terminal.write()`**: it bypasses the kernel's PTY line discipline by injecting bytes directly into the input queue (similar to the deprecated `TIOCSTI` ioctl) instead of writing to the PTY master file descriptor. This means:

- `Ctrl+C` (`\x03`) never generates SIGINT
- `Ctrl+Z` (`\x1a`) never generates SIGTSTP
- `Ctrl+\` never generates SIGQUIT
- The shell reports "no job control"
- TUIs that depend on proper signal handling break

### Upstream tracking

- Issue: [oven-sh/bun#25779](https://github.com/oven-sh/bun/issues/25779) (OPEN)
- Fix PR: [oven-sh/bun#25834](https://github.com/oven-sh/bun/pull/25834) — referenced by Jarred Sumner (OPEN, unmerged as of 2026-03-13)
- Related PR: [oven-sh/bun#26008](https://github.com/oven-sh/bun/pull/26008) (OPEN, unmerged as of 2026-03-13)
- Plexi is on Bun v1.3.10. The fix has not landed in any released version yet.

### What does NOT fix this

- **Switching xterm.js to libghostty**: libghostty replaces the frontend renderer/emulator but the bug is on the backend write path. The same broken `Bun.Terminal.write()` would still be used.
- **Frontend keybinding hacks**: Intercepting Ctrl+C in the frontend and sending `\x03` still hits the same broken write path.

## Resolution Paths (ordered by preference)

### Option 1: Wait for Bun upstream fix (recommended)

PR #25834 adds proper signal character detection via `TIOCSIG` ioctl. When merged, `Bun.Terminal.write()` will correctly generate SIGINT/SIGTSTP/SIGQUIT through the PTY line discipline. Zero code changes needed in Plexi.

**Action**: Monitor the PR. After it lands, upgrade Bun and verify Ctrl+C, fzf, and zoxide work.

### Option 2: Switch to `bun-pty` (if upstream stalls)

[bun-pty](https://github.com/sursaone/bun-pty) is a third-party package using Bun FFI to call Rust's `portable-pty` (from WezTerm). It writes to the PTY master fd properly, bypassing the `Bun.Terminal.write()` bug entirely.

Pros:
- Fixes the problem immediately
- Cross-platform (macOS, Linux, Windows via ConPTY)

Cons:
- Adds an FFI + Rust native dependency
- Small third-party package — maintenance risk
- Requires refactoring `session-manager.ts`

The swap would be localized to `src/bun/session-manager.ts`. The RPC layer, frontend, and xterm.js stay untouched.

### Option 3: Thin FFI wrapper around forkpty/openpty

Build a minimal Bun FFI wrapper around POSIX `forkpty`/`openpty`. Same result as option 2 but more DIY and no Rust dependency. macOS/Linux only.

## What the original 5-phase plan got right

- The `PtySession` interface abstraction is a reasonable decoupling if we ever need to swap PTY backends.
- The acceptance criteria (Ctrl+C works, fzf works, zoxide works, no protocol leaks) are correct and should be used to verify any fix.
- The advice about frontend shortcut conflicts is valid independently — global app shortcuts should not consume terminal keystrokes.

## What the original 5-phase plan over-scoped

The full 5-phase refactor (new PTY abstraction, replace Bun.Terminal, rewrite RPC bridge semantics, audit all frontend input, build diagnostic test harness) was designed around the incorrect "double emulation" diagnosis. Since the actual problem is a single bug in `Bun.Terminal.write()`, most of that work is unnecessary.

If the upstream fix lands, the only work needed is `bun upgrade`.

## Acceptance Criteria

The fix is complete when all of the following are true:

1. `Ctrl+C` reliably interrupts foreground processes and returns to a clean prompt.
2. `fzf` opens, responds to input, and exits without leaving stray escape sequences in the shell.
3. `zoxide zi` works without hanging Plexi.
4. Cursor-position-report-driven tools do not inject protocol responses into the shell command line.
5. Alternate-screen applications restore the normal prompt correctly.

## note-pty and Bun

`node-pty` does NOT work with Bun. It depends on V8 C++ APIs (Nan) and libuv symbols that Bun does not implement. The node-pty maintainers have marked Bun support as out-of-scope ([node-pty#632](https://github.com/microsoft/node-pty/issues/632)).

## Recommended Future Follow-up

If Plexi later adopts `libghostty`, keep the same separation of concerns:

- one renderer/emulator layer (libghostty or xterm.js)
- one PTY transport layer (Bun.Terminal or alternative)

The libghostty migration is orthogonal to this bug fix and should be evaluated on rendering fidelity and performance merits, not as a fix for signal handling.
