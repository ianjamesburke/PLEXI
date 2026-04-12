# Focus Manager — Priority-Aware Task & Attention App

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Pane tree model, pane naming, urgency hints  
**Ships with:** Plexi (built-in or bundled first-party app)

---

## Summary

A Plexi app that tracks what you're working on, knows what's most important, and gently redirects your attention when you've drifted too far from high-priority work. It combines pane naming, task priority, attention time tracking, and interrupt timing into one system.

This is not a to-do list. It's a priority-aware attention manager that uses Plexi's pane model as the tracking surface — because Plexi already knows which pane has focus, it already has the data.

---

## The Problem

You have 4 panes open. One is the critical bug fix (P1). One is a spec you started writing (P3). You double-click a file to check something in the P3 spec and 20 minutes later you're deep in a rabbit hole that isn't urgent.

No tool today catches this. Task managers are passive lists. Tiling WMs don't know about priority. The data (which pane is focused, for how long) exists in Plexi but isn't used.

---

## Core Concepts

### Pane Labels

Every pane can have a **label** — a short name that describes what's happening there. Labels are:

- Set manually by the user (Cmd+Shift+L → rename modal)
- Auto-suggested by Plexi based on context (CWD, app type, running command)
- Displayed in the pane's title bar area (already exists in Plexi's chrome)

Labels are not just cosmetic. They're the task name for the Focus Manager.

### Priority Levels

Four levels, matching the existing GitHub issue labels in the Plexi repo:

| Level | Meaning | Color |
|-------|---------|-------|
| P1 | Must finish today. Shipping blocker. | Red |
| P2 | Important, do next. | Orange |
| P3 | Nice to have. Background work. | Blue |
| P4 | Exploratory. Idle time only. | Grey |

Default: **P3** (everything starts as "nice to have" unless you say otherwise).

Priority is set per pane label — so if you have a group of linked panes all labeled "bug fix #42," they share one priority level.

### Attention Timer

Plexi tracks focus time per labeled context:

```
bug-fix-#42 (P1)     — 12 min today, currently focused
pyflow-spec (P3)     — 38 min today
snake-game (P4)      — 8 min today
```

The timer ticks when a pane with that label has focus. It pauses when focus moves elsewhere. This is trivial for Plexi to implement — it already tracks focused pane.

---

## Interrupt Logic

The hardest design problem. Research says:

1. **23 minutes** to regain context after a bad interrupt.
2. **20-25 minutes** is the minimum time before any nudge is justified.
3. **2+ priority level gap** is the minimum delta that justifies interrupting.
4. **Break boundaries** (natural pauses) are the only low-cost interrupt points.

### Rules

The Focus Manager uses these rules to decide when and how to surface a nudge:

**Rule 1: Minimum drift time.** Don't interrupt until you've been off-task for at least **20 minutes.** If someone is 5 minutes into a P3 detour with a P1 waiting, that's fine — they might be grabbing context. Wait.

**Rule 2: Priority delta threshold.** Only nudge when the gap between what you're doing and what's waiting is **≥ 2 levels.**

| Currently doing | Highest waiting | Nudge? |
|-----------------|-----------------|--------|
| P3 | P1 | Yes (gap = 2) — after 20 min |
| P3 | P2 | No (gap = 1) — ambient indicator only |
| P4 | P2 | Yes (gap = 2) — after 20 min |
| P2 | P1 | No (gap = 1) — ambient indicator only |

**Rule 3: Escalation ladder.** Nudges escalate gradually, never start aggressive:

| Stage | Trigger | Signal |
|-------|---------|--------|
| **Ambient** | Always visible | The P1 pane's border glows its priority color (red). Visible in peripheral vision but not disruptive. |
| **Soft nudge** | 20 min off-task, gap ≥ 2 | A small, non-modal notification appears in the Focus Manager pane: "P1: bug fix #42 waiting — 20 min since last focus." No sound. No overlay. |
| **Firm nudge** | 35 min off-task, gap ≥ 2 | The notification becomes slightly more prominent — maybe it pulses once, or the P1 pane's border pulses. Still no modal, no sound. |
| **Hard interrupt** | Never automatic | The system never forcibly switches your focus. The user is always in control. |

**Rule 4: Snooze and acknowledge.** Any nudge can be:
- **Snoozed** (15 min) — "I know, I'll get to it"
- **Acknowledged** — "I'm intentionally doing this lower-priority thing right now" (resets the timer, won't nudge again for 45 min)
- **Re-prioritized** — "Actually, this P3 is more important than I thought" (change its priority inline)

**Rule 5: No interrupts during input.** If the user is actively typing (keystrokes in the last 10 seconds), delay the nudge until there's a pause. Never interrupt mid-thought.

---

## The Rename + Priority Modal

Triggered by **Cmd+Shift+L** on any pane:

```
┌──────────────────────────────────┐
│  PANE LABEL                      │
│  ┌────────────────────────────┐  │
│  │ bug fix #42                │  │
│  └────────────────────────────┘  │
│                                  │
│  PRIORITY                        │
│  [P1] [P2] [P3] [P4]            │
│   ●         ○    ○    ○         │
│                                  │
│  GROUP                           │
│  ┌────────────────────────────┐  │
│  │ (none) ▼                   │  │
│  └────────────────────────────┘  │
│                                  │
│           [Save]  [Cancel]       │
└──────────────────────────────────┘
```

- **Pane label**: free text, autocompletes from existing labels (so you can assign a new pane to an existing context).
- **Priority**: four buttons, radio-style. Click to set. Shows the color.
- **Group**: optional dropdown of existing linked pane groups. Assigning to a group means this pane joins that group's visual outline and lifecycle.

Quick access: **Cmd+1/2/3/4** while the modal is open sets priority instantly.

---

## Focus Manager App (The Dashboard)

A Plexi app that shows the current state of all labeled contexts. Lives in a pane — typically a narrow sidebar or bottom panel.

### Layout

```
FOCUS MANAGER
─────────────────────────
● P1  bug fix #42           12m ← active
  P2  deploy pipeline        0m
  P3  pyflow spec           38m   ⏸ 20m ago
  P4  snake game             8m   ⏸ 1h ago
─────────────────────────
TODAY: 58m tracked

[!] bug fix #42 is P1 — last focused 0m ago
```

- Each row: priority dot (colored), label, time spent today, last-focused indicator.
- Active context is highlighted.
- Contexts are sorted by priority, then by last-focused (most recent at top within same priority).
- Total tracked time at bottom.

### Interactions

| Action | Effect |
|--------|--------|
| Click a row | Focus the pane with that label (Plexi switches focus) |
| Right-click a row | Change priority, rename, snooze, remove |
| `n` | Create a new labeled context (opens a pane and names it) |
| `d` | Mark a context as done (closes its pane group, logs completion time) |
| `h` | Toggle showing completed contexts from today |

### Compact Mode

For narrow panes, collapses to just priority dots and labels:

```
● bug fix #42    12m ←
  deploy          0m
  pyflow spec    38m
  snake            8m
```

---

## Pane Chrome Integration

Even without the Focus Manager app open, priority metadata is visible in Plexi's pane chrome:

- **Priority dot** in the pane title bar (left of the label). Red/orange/blue/grey.
- **Subtle border tint** — P1 panes have a very faint red border at all times. Just enough to draw the eye without being garish.
- **Attention badge** — when a nudge is active, a small pulsing dot appears on the P1 pane's title bar. Clicking it focuses that pane.

This means the Focus Manager app is optional for power users who just want the priority dots and ambient nudges. The app adds the dashboard, time tracking detail, and history.

---

## Idle Scavenging

Borrowed from CPU scheduling. When you're between tasks (no pane has been focused for > 30 seconds, or you're staring at the Focus Manager itself):

- The Focus Manager auto-suggests the highest priority context: "Ready to pick up bug fix #42?"
- If all P1/P2 work is snoozed or acknowledged, it suggests the next P3.
- P4 tasks are only suggested when nothing else is waiting.

This is the "idle time goes to low priority" pattern. The system naturally pushes you toward the most important unfinished work during transition moments, without interrupting flow on anything.

---

## Data Model

```
Context {
    label: String,
    priority: P1 | P2 | P3 | P4,
    pane_ids: Vec<PaneId>,          // panes with this label
    group_id: Option<GroupId>,      // linked pane group
    time_today_ms: u64,             // total focus time today
    last_focused: Timestamp,        // when focus last left this context
    created: Timestamp,
    snoozed_until: Option<Timestamp>,
    acknowledged: bool,             // user said "I know, not now"
    completed: Option<Timestamp>,
}
```

Persisted to `~/.plexi/focus.json` (or `~/.plexi-alpha/focus.json`). Resets daily at a configurable time (default: 4am, so late-night sessions aren't split at midnight).

---

## Manifest

```toml
[app]
id = "focus-manager"
name = "Focus Manager"
version = "0.1.0"
description = "Priority-aware task tracking and attention management"

[capabilities]
# Read-only — it observes pane state from Plexi, doesn't need filesystem or network

[app.builtin]
# Ships with Plexi. Has access to internal pane focus events
# that third-party apps don't get.
pane_events = true
```

### Builtin Privilege

This app needs data that third-party apps shouldn't get:
- Which pane has focus (attention tracking = surveillance if abused)
- Pane labels and groups
- Focus timestamps

This should be a **builtin** or **first-party trusted** app, not something the SDK exposes generally. The pane event stream is internal to Plexi.

---

## Integration Points

### With Pane Tree (pane-tree.md)

The Focus Manager reads the pane tree to understand grouping. When you label one pane in a group, all panes in that group inherit the label and priority. Collapsing a branch in the pane tree is reflected in the Focus Manager as "paused."

### With File Explorer + Text Editor (file-text-editor.md)

When the file explorer spawns a text editor via `open_pane`, the new pane inherits the parent's label and priority. No extra setup — the file you opened is part of the same context.

### With PyFlow

PyFlow panes labeled with a priority integrate naturally. A P1 PyFlow session gets the same nudge behavior as anything else.

### With Agents (future)

When an agent head spawns work in a pane, it can set the priority level. Agent-created P1 work gets the same ambient urgency signals as user-created P1 work. The Focus Manager doesn't care who created the context — just what priority it has.

---

## MVP Scope

1. **Pane labeling** (Cmd+Shift+L modal) — name any pane, set priority P1-P4.
2. **Priority dots in pane chrome** — colored dots in title bar based on priority.
3. **Attention timer** — track focus time per labeled context.
4. **Focus Manager app** — dashboard showing all contexts, sorted by priority, with time spent.
5. **Soft nudge** — non-modal notification after 20 min off a P1 task when doing P3/P4 work.

**Defer:** Firm nudge escalation, snooze/acknowledge, idle scavenging suggestions, history/completed view, daily reset, agent integration, group inheritance of labels.

---

## Open Questions

1. **Where does the Focus Manager pane live by default?** Bottom bar? Right sidebar? Overlay you summon with a keybinding? Recommendation: keybinding toggle (Cmd+Shift+F) that opens it as a narrow right sidebar. Not always visible — you summon it when you want the overview.

2. **Should completed contexts log anywhere persistent?** A daily summary of "P1: bug fix #42 — 47 min, completed at 3:12pm" could feed into the DEV_LOG or a dedicated focus log. Useful for retrospectives but not MVP.

3. **Multi-day tasks.** A P1 that spans three days — does the timer accumulate across days or reset daily? Recommendation: reset daily (today's focus), but show a "total" somewhere for multi-day contexts. The daily reset keeps the numbers actionable ("I've spent 2 hours on this today") rather than demoralizing ("I've spent 14 hours on this bug").
