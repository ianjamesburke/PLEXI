# Spatial Group Splits Implementation Plan

## Purpose

This document defines the recommended next-stage plan for adding tmux-like local density to Plexi without breaking the overview, the spatial model, or future multi-surface expansion.

It is intentionally written as an execution plan for parallel exploration across separate branches or worktrees.

## Why This Is the Right Next Step

Plexi already has the important foundation:

- a 2D top-level workspace model
- directional focus based on spatial neighbors
- contexts
- minimap and overview
- workspace persistence

The missing piece is local composition inside a single canvas position.

The README calls this out directly as:

- "Pane Management: Considering tmux-style split pane management within a single canvas node."

The key design constraint is that Plexi must remain spatially truthful.
If the user cannot trust what the overview means, Plexi stops feeling like a serious replacement for `tmux` plus a 2D window manager.

## Recommendation

Adopt **top-level spatial groups with internal splits**.

Do **not** make tabs the primary next abstraction.
Do **not** introduce arbitrary recursive nested trees in the first mergeable version.

### Recommended model

- the 2D canvas remains the only top-level truth
- each top-level node is either:
  - a single pane
  - a split-group
- a split-group contains `2-4` child panes in a small local layout
- the overview shows the group as one node
- internal split state is secondary detail revealed in focus view, not primary minimap structure

### Why this beats tabs-first

- tabs hide spatial state instead of clarifying it
- tabs make the overview dishonest by omission
- tabs solve hiding and density better than they solve grouping
- tabs can still be added later inside a group if they prove necessary

### Why this beats full recursive trees in v1

- recursive trees create a second layout system immediately
- the overview becomes much harder to explain
- persistence and keyboard navigation complexity grow too quickly
- a one-level group model is enough to test whether the concept is genuinely good

## Product Model

### New top-level concept

Introduce a `group` as a top-level workspace node.

Recommended hierarchy:

- `Context`: current task lane
- `Node`: top-level object on the 2D canvas
- `Pane`: child surface inside a node

In the current app, "panel" is doing both top-level and leaf duties.
This feature should separate those roles conceptually, even if implementation stages keep some existing names during migration.

### Node types

Top-level nodes should support:

- `single`: exactly one child pane
- `split-group`: small child layout with one active child

Child panes should keep the existing pane-type direction:

- `terminal`
- future `browser`
- future `excalidraw`
- future `notes/reference`

This is important. The split model should be designed as a general surface container, not a terminal-only workaround.

## UX Rules

### Overview and minimap

The overview continues to represent **top-level nodes**, not every child pane.

Each node should show:

- active child title or group label
- a compact occupancy indicator
- active/focused state

Optional but recommended later in the stage:

- tiny subdivision hint inside the node tile to suggest split shape
- child count badge

The overview must remain glanceable and truthful.

### Focus and navigation

Navigation should be two-layered but predictable:

- directional focus first tries to move within the active group
- if there is no child in that direction, it escapes to the nearest top-level node in that direction
- top-level spatial movement remains the canonical fallback

This preserves the existing mental model instead of replacing it.

### Split creation

Recommended commands:

- split right inside current node
- split down inside current node
- optionally "pop out to top-level node" later

When a single node is split:

- it is converted in place into a split-group
- the current pane becomes one child
- the new pane becomes the adjacent child
- the node's canvas position does not change

### Closing

When a child pane closes:

- if multiple children remain, the group persists
- if only one child remains, the group collapses back to a `single` node

### Tabs

Tabs should be explicitly out of scope for the first stage.

If later added, they should live inside a node as a secondary stack mechanism, not replace the primary split-group model.

## What Fits in This Stage

These items can reasonably fit in the same stage because they directly support the split-group model.

### Must-have

1. top-level node vs child pane model
2. one-level split-groups with `right` and `down` creation
3. overview/minimap representation of groups as top-level units
4. focus and directional escape rules between child panes and neighboring nodes
5. persistence of group structure and active child
6. close/collapse behavior for groups

### Strongly recommended in the same stage

1. manual pane labels
2. group label or derived title
3. quick "jump back" between last focused panes
4. context reordering and pinning

Why these fit:

- labels make grouped nodes legible in overview
- jump-back offsets extra navigation depth
- context ordering is low-cost and improves the larger workspace story immediately

### Optional stretch items

1. lightweight layout undo / reopen last closed pane
2. compact group outline in focus view
3. child-count or split-shape badges in minimap

## What Should Stay Out of This Stage

These should not be squeezed into the same implementation push.

- multi-workspace storage redesign
- worksheets/templates
- browser panes
- Excalidraw panes
- full command palette
- SSH
- background session persistence daemon
- `libghostty`
- recursive nested split trees
- tab stacks

Reason:

This stage should prove the split-group model first.
Mixing storage redesign, new pane surfaces, and session architecture changes into the same effort will make it impossible to evaluate what actually worked.

## Branch / Worktree Strategy

Use separate worktrees to compare the interaction model directly.

Recommended worktree set:

### 1. `codex/spatial-groups-core`

This is the recommended merge candidate.

Scope:

- one-level split-groups only
- no tabs
- no recursive nesting
- overview shows groups as single nodes
- focus escape rules implemented
- persistence implemented
- manual pane labels included

This branch answers the core product question:
"Does local grouping improve Plexi without breaking the spatial truth?"

### 2. `codex/spatial-groups-recursive`

This is a contrast branch, not the default merge target.

Scope:

- recursive split trees allowed inside a top-level node
- same overview rule as core branch
- same persistence requirements

Purpose:

- test whether recursive power is actually worth the complexity
- validate whether keyboard navigation and overview comprehension degrade

Expected outcome:

- useful for learning
- likely too complex for the first merge

### 3. `codex/spatial-groups-with-stacks`

This is the tabs/hidden-stack comparison branch.

Scope:

- one-level split-groups
- optional stack behind a child or node
- overview still only shows top-level nodes

Purpose:

- test the "hide sessions behind panes" idea directly
- measure whether stacks feel powerful or just obscure

Expected outcome:

- likely weaker than the core branch for spatial clarity
- may validate tabs as a later secondary feature

## Evaluation Criteria Across Worktrees

Each branch should be judged against the same questions:

1. Can a new user explain what the overview is showing after five minutes?
2. Can the user reliably predict where directional focus goes?
3. Does saving and restoring grouped layouts feel trustworthy?
4. Does the model still feel like one workspace, not two overlapping layout systems?
5. Does it improve real workflows with `2-4` related terminals, not just demos?

If a branch fails any of those, it should not merge regardless of how powerful it seems.

## Implementation Stages

### Stage 1. State model refactor

Introduce an explicit top-level node model.

Required state shape changes:

- top-level canvas entities should carry position and context membership
- child panes should carry pane type, title/label, session metadata, and local relationship inside the node
- one active node per context
- one active child per active node

Migration direction:

- existing panels become `single` nodes during migration
- existing persisted workspaces should load losslessly into the new format

### Stage 2. Rendering model

Update focus rendering so the active node can render either:

- a single active pane
- a split-group container with child panes

The active child should still own normal keyboard input.
Non-active children inside the same node should be visible and focusable, but only one child should be interactive at a time.

### Stage 3. Overview model

Update minimap and overlay overview to operate on top-level nodes instead of leaf panes.

Required behavior:

- one node marker per top-level node
- active child information summarized in the node marker
- click focuses the node, then the active child inside it
- node movement and spatial relationships remain top-level concerns

### Stage 4. Commands and keyboard rules

Add split-group commands while preserving current workspace rules.

Recommended commands:

- create split right in node
- create split down in node
- focus child left/right/up/down
- move to neighboring top-level node when no child exists in that direction

Modifier discipline from the MVP interaction spec should remain intact:

- unmodified typing belongs to the active terminal
- workspace commands remain modifier-based

### Stage 5. Persistence and restore

Persist:

- node type
- node position
- child pane list
- active child
- pane labels
- split orientation/arrangement

Restore rules:

- old workspaces migrate to single nodes
- grouped workspaces restore without flattening
- groups with only one child normalize back to `single` on save or load

### Stage 6. Polish items in-stage

If the core branch is working well, add:

- pane labels
- context reorder/pin
- jump-back

Do not add optional stretch items until core navigation and restore are clearly stable.

## Testing Plan

### Unit and state tests

- migrate old flat panel state into new node model
- split a single node and persist the result
- close a child pane and collapse back to single-node form
- directional navigation within a group behaves correctly
- directional escape from group to neighboring node behaves correctly
- active child and active node restore correctly per context
- context reorder/pin does not corrupt grouped layouts

### E2E tests

- create split right and split down from an active terminal
- switch between child panes using keyboard commands
- move out of a group to a neighboring node with directional navigation
- save, reload, and verify grouped layout is restored
- click node in minimap and verify correct node/child focus
- close child panes until group collapses back to single
- verify standard terminal input still works inside active child

### Manual product checks

- build a `2x2` project area and evaluate whether it feels better than multiple top-level nodes
- verify overview still reads clearly at `6-12` top-level nodes
- verify labels are enough to keep grouped nodes understandable

## Success Criteria

This stage is successful if:

1. grouped splits feel like an extension of the current spatial model, not a competing one
2. the overview remains easy to read and truthful
3. users can manage `2-4` related panes locally without getting lost
4. persisted grouped layouts restore reliably
5. the model cleanly leaves room for future browser, notes, and whiteboard panes

## Decision Log

- Chosen: top-level spatial groups with internal splits as the primary next direction.
  Reason: best balance of local density, overview clarity, and future extensibility.
- Rejected for v1: tabs as the primary organizational model.
  Reason: tabs hide too much state and weaken overview truth.
- Rejected for v1: recursive nested split trees as the merge target.
  Reason: too much complexity before the basic group model is validated.
- Chosen: run competing implementations in separate worktrees.
  Reason: lets the product be judged by feel, not just by theoretical cleanliness.
