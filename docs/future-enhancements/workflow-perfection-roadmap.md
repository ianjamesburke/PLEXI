# Workflow Perfection Roadmap

## Purpose

This document turns the existing future enhancements into a more opinionated roadmap focused on workflow quality.

It is intentionally biased toward:

- daily usability gains over flashy surface area
- features that build on Plexi's current context and persistence model
- sequencing that reduces rework

## What Exists Today

Based on the current codebase and docs:

- Plexi already has named contexts, context switching, context rename/delete, and per-context focused-panel memory.
- Workspace state is already persisted locally.
- Persistence currently centers on a single default workspace document on disk.
- There is already a concept of storage profiles (`default`, `clean`), but not user-facing multi-workspace management.
- Future docs already exist for browser panes, Excalidraw panes, PTY improvements, and live edge previews.

That means the highest-leverage near-term ideas are mostly not "invent workspace from scratch." They are about making the current context/workspace model feel much more intentional and less fragile.

## Planning Assumptions

- "Worksheet" in this document means a named saved arrangement of contexts, panels, and metadata that can be restored later.
- "Workspace" means a broader container that can hold one or more contexts and, later, multiple saved worksheets.
- Terminal-first workflows remain the product center of gravity.
- Remote/agent-heavy orchestration should come after the local workflow model feels solid.

## Prioritization Heuristic

Features are ordered by:

1. user value per unit of implementation effort
2. how much they clarify the mental model
3. how strongly they unlock later features
4. how much they avoid future migration pain

## Recommended Order

### 1. Context reordering and pinning

**Why first**

Contexts already exist, so this is pure workflow polish with almost no product-model risk. Reordering fixes a real daily irritation immediately and helps users shape their environment around active work instead of creation order.

**What it should do**

- drag or keyboard-move contexts left/right
- pin a few contexts to the front
- keep stable shortcuts for pinned/favorite contexts where possible
- preserve order across relaunch

**Complexity**

`Low`

**Why complexity is low**

- existing context state already lives in one array
- persistence is already in place
- this is mainly UI, command, and serialization work

**Risks**

- keyboard shortcuts become ambiguous if order changes carelessly
- index-based assumptions may need to be replaced with ID-based logic in a few places

### 2. Saved worksheets within the current workspace

**Why second**

This is the most obvious "workflow perfection" win. Users often want a few repeatable views for the same project: coding, debugging, deploy, research, review. It adds real leverage without forcing a full multi-workspace architecture immediately.

**What it should do**

- save the current arrangement as a named worksheet
- restore a worksheet quickly
- optionally duplicate from the current state
- optionally mark one worksheet as the default for the current workspace

**Complexity**

`Medium`

**Why complexity is medium**

- state snapshotting already exists conceptually via workspace persistence
- the main work is turning one live document into a small library of named saved states
- UI and conflict behavior need design discipline

**Risks**

- users may confuse contexts vs worksheets unless names are clear
- saving full session state vs saving just layout needs explicit product rules

### 3. "Save worksheet as template" and quick-start presets

**Why third**

Once worksheets exist, templates become cheap and very powerful. This is the first feature that starts making Plexi feel like a real system rather than just a persistent layout tool.

**What it should do**

- save a worksheet as a reusable starter
- create new project setups from templates
- support common presets like "build + editor agent + docs + logs"
- optionally parameterize working directories later

**Complexity**

`Medium`

**Why complexity is medium**

- it reuses the worksheet format
- most of the work is metadata, creation flow, and defaults
- later parameter support raises complexity, but v1 does not need it

**Risks**

- overdesigning template variables too early
- mixing project bootstrap concerns with terminal session lifecycle

### 4. Workspace switcher (multiple workspaces)

**Why fourth**

This is worth doing, but only after Plexi proves the lighter-weight worksheet layer. Otherwise there is a real chance of building workspaces when the product actually needed better saved states inside one workspace.

**What it should do**

- let users create and switch between multiple named workspaces
- keep each workspace's contexts and worksheets isolated
- support "recent workspaces" and one default startup target

**Complexity**

`Medium-High`

**Why complexity is medium-high**

- storage moves from one canonical file to a small workspace registry
- startup, switching, and deletion flows need careful handling
- this becomes a foundational persistence and identity change

**Risks**

- migration from today's single default workspace
- accidental data loss if switching or deletion is not designed safely

### 5. Reopen closed pane and lightweight layout undo

**Why fifth**

This is classic quality-of-life tooling. It reduces fear, encourages faster navigation, and makes the spatial canvas feel much less brittle.

**What it should do**

- reopen the most recently closed pane in place if possible
- provide 1-step or small-stack undo for layout actions
- restore focus intelligently

**Complexity**

`Medium`

**Why complexity is medium**

- requires action history and restore semantics
- panel layout restore is straightforward compared with terminal process restore
- session resurrection rules must be explicit

**Risks**

- users may expect process state resurrection when only layout can be restored
- undo stacks can get messy across context switches if not scoped clearly

### 6. Jump history and quick switcher

**Why sixth**

Once contexts and worksheets multiply, spatial navigation alone stops being enough. A fast "go back / jump to recent / fuzzy-find context or pane" layer becomes necessary.

**What it should do**

- jump back to the previously focused pane
- open a quick switcher for contexts, worksheets, and panes
- show recent locations and perhaps active process names

**Complexity**

`Low-Medium`

**Why complexity is low-medium**

- focus history is cheap to record
- the main effort is command palette UI and ranking rules

**Risks**

- search results get noisy if labels are weak
- command palette can become a junk drawer if scope is not constrained

### 7. Session labels, statuses, and attention badges

**Why seventh**

This is the first major step toward the agent-orchestration future without requiring full notification routing. It also helps normal human workflows by making panes legible.

**What it should do**

- allow manual pane labels
- expose light status states like running, idle, waiting, error
- show "needs attention" badges when a pane likely wants user input

**Complexity**

`Medium`

**Why complexity is medium**

- labels are easy
- useful attention detection is harder and may need shell heuristics
- UI has to stay quiet enough not to feel noisy

**Risks**

- false positives make the feature feel unreliable
- heuristic status detection can become platform-specific

### 8. Per-project startup rules

**Why eighth**

This is where Plexi starts compounding value: open project X, get the right worksheet/template, cwd, and maybe pane labels automatically. It makes persistence feel alive instead of archival.

**What it should do**

- map a repo or folder to a preferred worksheet/template
- restore the last-used setup for that project
- optionally run a small startup sequence later

**Complexity**

`Medium`

**Why complexity is medium**

- mostly metadata and launch-routing logic
- can build on top of worksheets/templates rather than inventing a new model

**Risks**

- path identity gets tricky across moved repos or symlinks
- automatic startup behavior can feel magical in a bad way if not visible

### 9. Notes/reference pane before heavier embedded surfaces

**Why ninth**

A lightweight notes or markdown pane likely delivers more daily value sooner than a full browser or whiteboard, and it teaches the codebase how to support non-terminal surfaces with less complexity.

**What it should do**

- open a simple markdown or plaintext notes pane
- persist pane content locally
- support project checklists, commands, and reminders

**Complexity**

`Medium`

**Why complexity is medium**

- simpler than browser panes
- still requires formalizing mixed panel-type rendering and persistence

**Risks**

- half-baked editing UX if the surface is too minimal
- could overlap with external editor workflows unless kept lightweight and tactical

### 10. Browser pane with Plexi-managed profiles

**Why tenth**

This is a strong feature, and the supporting doc is already good. It just should not outrun the core workflow model. Browser panes become much more valuable once workspaces, worksheets, and templates are in place.

**Complexity**

`Medium-High`

**Why complexity is medium-high**

- first heavy non-terminal surface
- persistence, focus, and profile isolation add real platform complexity

**Dependencies**

- multiple panel types should feel structurally normal first
- workspace/worksheet persistence should already be stable

### 11. Excalidraw pane

**Why eleventh**

This is compelling, but slightly more niche than notes and browser support. It also benefits from the same mixed-pane groundwork.

**Complexity**

`Medium`

**Why complexity is medium**

- panel-type plumbing is shared with other pane types
- embedding and persistence are conceptually simpler than browser profiles

**Dependencies**

- mixed panel-type model
- stable pane focus behavior

### 12. True session persistence and background multiplexing

**Why twelfth**

This is a major strategic unlock, but it should come after the workspace model is mature enough to deserve durable background sessions. Otherwise the architecture may solidify around the wrong user abstractions.

**What it should do**

- keep PTYs alive when the UI closes
- restore UI attachment to those sessions later
- become the base for SSH pooling and richer orchestration

**Complexity**

`High`

**Why complexity is high**

- daemon or background service model
- lifecycle and failure semantics become much harder
- major testing and platform behavior burden

**Risks**

- process leaks, zombie sessions, and shutdown edge cases
- users will expect tmux-grade reliability once this exists

### 13. SSH-backed sessions and hybrid local/remote workflows

**Why thirteenth**

Remote support matters, but it should sit on top of stable session persistence rather than inside today's simpler local-session model.

**Complexity**

`High`

**Why complexity is high**

- credential handling
- reconnect behavior
- host metadata
- session pooling and failure recovery

### 14. Agent orchestration features

**Why fourteenth**

These are central to the longer-term vision, but they should arrive after the workspace model, pane metadata, and session durability are already trustworthy. Otherwise the user ends up babysitting the orchestration system itself.

**What it should do**

- attention routing
- interrupted-work summaries
- jump-to-waiting-session actions
- grouped agent task views

**Complexity**

`High`

**Why complexity is high**

- depends on status semantics, durable sessions, and clear workspace identity
- a lot of UX complexity hides behind "just show badges"

### 15. libghostty migration

**Why later**

Important, but strategically separate. It improves rendering and compatibility, yet it is not the main thing making the workflow feel polished today.

**Complexity**

`High`

**Why complexity is high**

- native integration
- cross-platform packaging
- rendering parity and interaction parity work

### 16. Workspace families, stacks, or higher-order grouping

**Why last**

This is the most speculative item. It may be genuinely useful, but it is also the easiest place to overbuild. A lot of the underlying need may already be solved by better worksheets, templates, and multi-workspace switching.

**What it might mean**

- a parent grouping above workspaces
- client-based or company-based workspace collections
- batch open/close flows for related work

**Complexity**

`High`

**Reason to defer**

- the product probably needs more real-world usage before this abstraction is trustworthy

## Condensed Roadmap

### Best next 5

1. Context reordering and pinning
2. Saved worksheets
3. Worksheet templates / presets
4. Multiple workspaces
5. Reopen closed pane + layout undo

### Best next 5 after that

1. Jump history and quick switcher
2. Session labels and attention badges
3. Per-project startup rules
4. Notes/reference pane
5. Browser pane

### Later strategic bets

1. Excalidraw pane
2. Background session persistence
3. SSH and hybrid remote
4. Agent orchestration
5. libghostty
6. Workspace families

## Suggested Product Shape

If Plexi is going to feel "finished," the cleanest hierarchy is probably:

- `Workspace`: broad container for a domain of work
- `Worksheet`: named saved arrangement inside a workspace
- `Context`: active task lane inside a worksheet
- `Pane`: a terminal or other surface inside a context

That hierarchy is not fully present in the code today, but it is a useful target because it makes each concept do one job:

- workspaces separate larger domains
- worksheets capture repeatable arrangements
- contexts separate concurrent tasks
- panes hold the actual work surfaces

## Recommendation

If only three things are done next, they should be:

1. context reordering/pinning
2. saved worksheets
3. multiple workspaces

That sequence gives Plexi a clearer mental model fast, solves obvious workflow friction, and creates a stable base for everything more ambitious later.
