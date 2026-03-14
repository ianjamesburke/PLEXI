# Browser Pane and Profile Support

## Purpose

This document describes a future enhancement for adding a Chromium-based browser pane to Plexi, with optional support for custom/local profile data.

This is a **future enhancement** document, not MVP scope by default.

## Summary

Adding a browser pane is feasible and aligns with the future PRD direction.
Supporting arbitrary local Chrome profile directories is feasible but carries higher reliability and security risk.

Chosen default direction:

- Ship browser panes first with Plexi-managed isolated profiles
- Add optional profile import and migration in a later phase
- Avoid direct in-place mounting of a live user Chrome profile in v1

## Capability Options

### Option A: Browser pane + Plexi-managed profiles (recommended)

- Each browser pane/context uses a profile directory owned by Plexi.
- Profile lifecycle is controlled by Plexi (create, restore, rotate, delete).
- Supports cookies, session state, tabs/history local to Plexi.

### Option B: Browser pane + profile import from local Chrome

- User points to a local Chrome profile directory.
- Plexi copies selected data into a Plexi-owned profile.
- Import may be partial (cookies/history/preferences), with compatibility checks.

### Option C: Browser pane + direct use of local Chrome profile path

- Plexi uses the user’s existing Chrome profile directory in place.
- Highest user convenience, but highest breakage/security risk.
- Not recommended for initial release.

## Cost-Benefit-Risk Analysis

### Option A: Plexi-managed profiles

Cost:

- Medium implementation effort (pane lifecycle, persistence, profile storage plumbing).
- Medium test effort across macOS/Windows behavior.

Benefits:

- Predictable behavior and easier supportability.
- Lower risk of corrupting user’s primary browser profile.
- Clear isolation boundary per workspace/context.

Risks:

- Users cannot instantly reuse all existing Chrome state.
- Some feature parity gaps vs full desktop Chrome profile.

Risk level: **Low-Medium**
Delivery estimate: **~1-2 weeks** for an initial, stable implementation.

### Option B: Import/migration into Plexi-managed profiles

Cost:

- Medium-high implementation complexity (format/version checks, copy filters, rollback).
- Higher QA surface due to profile-version and OS-specific behaviors.

Benefits:

- Better onboarding for users with existing browser workflows.
- Preserves safety of Plexi-owned runtime profile after import.

Risks:

- Partial import failures and edge-case data incompatibility.
- User confusion if imported data differs from source profile.

Risk level: **Medium**
Delivery estimate: **~2-4 additional weeks** after Option A.

### Option C: Direct local Chrome profile usage

Cost:

- High support and reliability burden.
- High debugging overhead for profile lock/version/encryption problems.

Benefits:

- Maximum convenience in theory (existing cookies/extensions/preferences).

Risks:

- Profile corruption risk when Chrome and embedded Chromium versions diverge.
- Lock-file conflicts if Chrome is running.
- OS-protected secrets may fail to decrypt or behave inconsistently.
- Large security blast radius if a high-trust personal profile is reused.

Risk level: **High**
Delivery estimate: unpredictable; high operational risk even if implemented.

## Technical Implementation Direction

### Browser pane architecture

- Extend panel model to support `browser` as first-class in runtime rendering.
- Route panel surface rendering by type:
  - `terminal`: existing xterm + PTY bridge
  - `browser`: embedded Chromium view with pane-level lifecycle
- Keep PTY/session manager unchanged for non-terminal panels.

### Profile storage model

- Store browser profiles under a Plexi-owned root (for example, per workspace profile namespace).
- Track profile metadata in workspace state/config.
- Add profile integrity checks and fallback profile creation on load failure.

### Input and focus model

- Active browser pane owns pointer and normal typing input.
- Global workspace shortcuts remain modifier-based and consistent across pane types.
- Directional focus/navigation behavior stays shared with terminal panels.

### Persistence model

- Persist browser pane URL and profile binding metadata.
- On restore, recreate pane with same URL/profile mapping.
- Add explicit handling for missing/corrupt profile directories.

## Security and Reliability Notes

- Treat imported profile data as untrusted input.
- Prefer copy-on-import over in-place mutation.
- Never assume extension compatibility across Chromium versions.
- Provide safe-mode startup path for profile failures.
- Include clear UI copy when running with imported/custom profile data.

## Recommended Rollout

1. Phase 1: Browser pane with Plexi-managed profile only.
2. Phase 2: Controlled one-way import from local Chrome profile to Plexi profile.
3. Phase 3: Optional advanced profile tooling (backup/repair/reimport), if demand is high.
4. Explicitly defer direct in-place profile mounting unless strong demand and safeguards justify it.

## Acceptance Criteria

1. User can create, focus, move, and close a browser pane.
2. Browser pane state (URL + profile binding) restores across relaunch.
3. Terminal behavior and PTY lifecycle are unchanged.
4. Profile load failures recover safely without crashing workspace restore.
5. Cross-pane keyboard navigation remains consistent.

## Test Plan

### Unit and state tests

- Panel state serialization for `browser` panes round-trips correctly.
- Profile metadata validation rejects invalid/missing bindings.
- Restore path handles profile corruption/missing path via safe fallback.

### E2E tests

- Create browser pane, navigate URL, save, restart, verify restoration.
- Mixed terminal+browser workspace focus/navigation checks.
- Simulated profile load failure path verifies fallback and user-visible status.
- Browser pane interactions do not regress terminal input behavior.

### Platform checks

- Validate profile behavior on Windows and macOS separately.
- Validate migration/import flows against at least two Chrome profile versions.

## Non-Goals (for this enhancement)

- No cloud sync of browser profiles.
- No guarantee of full extension compatibility with desktop Chrome.
- No direct write access to a live primary Chrome profile in initial releases.
