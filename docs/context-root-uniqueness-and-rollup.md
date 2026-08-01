# Duplicate-Root Hard Stop + Parent-Dir Rollup — Design Brief

**Stint:** 0679
**Status:** active — pre-approval. No code, no stints filed; the stints that
would follow a ruling are described at the end.

Ian's ruling of record: no two contexts may share a root, enforced as a hard
stop at context creation with a clear error; and a parent directory's context
should roll up the todos of child-directory contexts. Both change what a
context *is*, so they get a written design checked against `NORTH_STAR.md`
before any code. This brief recommends one design for each, names what was
rejected, and lists the decisions that are Ian's to make.

The empirical half — whether duplicates are possible today and what they break —
is answered by the audit filed alongside this task,
[`context-state-persistence-audit.md`](context-state-persistence-audit.md).
Cited here, not re-derived.

## 1. What a context root is today

A root is a single directory a context is anchored to. It is the address every
context-scoped thing is derived from: the app registry scan, the workspace
config overlay, the agent set, `PLEXI_CONTEXT_ROOT` in every pane's
environment, notes scoping, and — the subject of the audit — app state paths.

**Every place a root is set or changed** — enumerated by grepping every write
to `Context::root`, not by following the entry points a reader would expect:

| Path | Root it assigns |
|---|---|
| `new_context_empty` (`plexi context create` with no root, ⌘-new-context) | the home directory, unconditionally |
| `new_context_at_path` (`context create --root`, open-folder-as-context) | the given path |
| `push_pane_to_subcontext` (push a pane into a sub-context) | **the parent context's `root`, verbatim** |
| `new_child_context` (sub-contexts, portals) | the path its caller passes — and `new_child_context_from_keyboard` passes the parent's **`path`**, not its `root` |
| `set_context_root` (`plexi context set-root`, ⇧⌘I, sidebar `SetRoot`, root-overlay commit with a non-empty path) | the given path, on one context, with no other checks |
| `WindowMenuAction::ClearRoot` (sidebar row menu) | **clears the root by direct field write**, bypassing `set_context_root` |
| Root-overlay commit with an empty buffer (`OverlayTarget::ContextRoot`) | **clears the root by direct field write**, likewise |
| Workspace restore | whatever the saved file holds |

Two of these deserve spelling out, because both were missed on the first pass
through this design and each is a hole a guard placed at the obvious entry
point would leave open.

**`Context::path` and `Context::root` are different fields, and nothing keeps
them in sync.** `set_context_root` writes only `root`; nothing writes `path`
after the context is created. So the keyboard sub-context path anchors the child
at wherever its parent was *created* — which after any set-root is a directory
the user deliberately moved away from. The audit reproduces this
(`audit_0678_keyboard_sub_context_inherits_the_parents_stale_path_not_its_root`).
A guard that only rejects *collisions* would wave this through: the child's root
is not a duplicate, it is simply wrong.

**Two paths clear the root by writing the field directly**, and both re-implement
`set_context_root`'s transition-effects tail inline rather than calling it. That
is the finding, not the typo: the choke point already exists and callers already
go around it, so a convention that says "route root changes through
`set_context_root`" is not a mechanism — it is a rule that has already been
broken twice in shipped code.

Stint 0651 (open on PR #2536, not merged) makes `Context::root` non-optional,
so "no root" stops being a state. It does not make roots unique, and it does
not close the direct-write paths.

**Are duplicates possible today?** They are not merely possible; two of the
paths above *guarantee* them. Every rootless new context lands on the home
directory, and a pushed sub-context inherits its parent's root by construction.
The
audit records a live saved workspace holding two contexts both rooted at the home
directory, and shows what that costs: a shared root means one shared
context-scoped state file per app, addressed identically from both contexts,
overwritten whole by whichever instance persists last. Nothing warns, and nothing
merges.

That is the case for the rule. It is also the reason the rule cannot simply be
switched on: **as literally stated, it outlaws the sub-context model**, which is
shipped behavior.

## 2. The hard stop

### Recommended design

**One guard, at one choke point — and the choke point must be enforced by the
compiler, not by convention.** Root assignment becomes a single fallible
operation on the router: `set_root` and `clear_root`, both returning a typed
rejection that names the conflicting context. Guarding only `plexi context
set-root`, or only creation, leaves most of the table open; the audit's evidence
is that the doors nobody thinks about (rootless create, sub-context inherit) are
the ones duplicates actually come through.

**`Context::root` becomes private to the router**, so those two methods are the
only way to write it and a direct `.root = …` is a compile error. This is the
part the first draft of this brief got wrong, and the correction is the whole
lesson: the recommendation was "one choke point", the choke point already
existed as `set_context_root`, and two shipped call sites already went around it
by writing the field directly. A guard reachable only by a caller who chooses to
call it does not survive its first busy afternoon. Encapsulation is what makes
the count of mutation sites stop mattering — it stays correct when the eighth
one is added by someone who never read this document.

**The two clear paths route through `clear_root`**, which also owns the
transition-effects tail both of them currently re-implement inline.

**`Context::path` is the other half of the same defect.** Either it is kept in
sync with `root` by the same operation, or the sub-context creation path stops
reading it and reads `root` instead. Recommendation: the latter — `path` is the
context's creation-time working directory and has no business anchoring a child.
This is the seventh open decision in §5.

**Sub-contexts are exempt, and the rule is restated to make that explicit:** *no
two **top-level** contexts may share a root; a sub-context shares its parent's
root by definition and is addressed as part of it.* This is the smallest change
that keeps the rule true and the shipped model intact — see the open decision in
§5 if Ian wants the stronger version instead.

**Error text** (CLI, on the rejected operation — never a silent no-op):

```
context root already in use by context "notes"
  requested: /Users/ianburke/Documents/notes
  in use by: context "notes" (id 14)
A root anchors exactly one context — app state, agents, and config all resolve
through it, so two contexts on one root overwrite each other's state.
Next: switch to it            plexi context switch notes
      or re-root this context plexi context set-root <other-dir>
      or take the root over   plexi context set-root <dir> --steal
```

The user's next move is always named, and one of the three is always right.
`--steal` clears the root from the other context and requires it to be re-rooted
before it resolves anything — deliberately louder than a silent swap.

**Existing duplicates when the rule turns on** — the sharp edge, and the place a
creation-only guard fails. A rule that guards only new assignments leaves exactly
the invalid state it exists to prevent, and the audit shows that state already
exists in a live workspace.

Recommendation: **load, flag, never silently mutate.** Workspace restore keeps
every context exactly as saved — a workspace that fails to load, or that
quietly re-roots the user's contexts at boot, is a far worse failure than the
duplicate. On detecting a duplicate at load, the host logs it at `warn`, marks
the affected contexts in the sidebar, and surfaces one resolution command
(`plexi context doctor`) that lists each conflict and the same three moves as
the error text. The rule is enforced strictly on every *new* assignment from
that moment, so the set of duplicates can only shrink.

### Rejected

- **Refuse to load a workspace containing duplicates.** Correct-by-construction
  and unusable: it bricks the user's session over a condition the host itself
  created.
- **Auto-migrate at load** (re-root duplicates to a generated subdirectory).
  Silently moves where a user's state resolves — the exact failure the audit
  documents, performed deliberately.
- **Warn only, never block.** This is today's behavior. The audit is the
  evidence that a warning nobody reads does not prevent the data loss.
- **Enforce in the CLI.** Leaves the GUI paths (⇧⌘I, sidebar, root overlay) and
  workspace restore unguarded. The router is the only place they all meet.
- **A guarded `set_context_root` that callers are expected to use.** This is
  today's shape, and two call sites already bypass it. Rejected on the evidence
  of its own track record.

### The comparison rule

Two roots are the same root when they name the same directory, not when their
strings match. Recommended, in order:

1. **Canonicalize both sides** (resolve symlinks and `..`) before comparing.
   Two contexts rooted at a symlink and its target are one root and must
   collide.
2. **A root that does not exist yet is rejected**, not created and not compared
   lexically. Root assignment already writes into the directory
   (`auto_init_workspace`), so a non-existent root is a typo far more often than
   an intent; the error names the missing path. This keeps canonicalization
   total — there is no "partially resolvable path" case to hand-roll.
3. **On macOS, compare case-insensitively.** The default volume format is
   case-insensitive, so two roots differing only in case *are* one directory and
   would share one state file. Comparing case-sensitively there would let the
   duplicate through the guard and produce exactly the bug the guard exists to
   prevent. Per-volume detection is the theoretically correct version and is not
   worth it: the cost of the blunt rule is a rejected duplicate on a
   case-sensitive volume, which the error text already tells the user how to
   resolve.
4. **Hard links and bind-mount-style aliases are out of scope.** They cannot be
   detected from the path, and no realistic user hits them.

Comparison lives next to the guard, so there is one answer to "are these the
same root" for the model, the CLI, and any future rollup ancestry check.

## 3. Parent-directory rollup

Ian's shape: a context whose root is an ancestor of another context's root sees
the descendants' todos, aggregated and attributed.

### Recommended design

**A read-only, host-computed view over registered contexts — not a merge, and
not a filesystem walk.**

- **What crosses:** nothing by default. An app opts into being rolled up by
  declaring it, and reads the aggregate through a host API that returns
  *attributed* entries (each item carries the context it came from). This keeps
  rollup inside the capability model — commandment 10 is that apps never get
  ambient authority, and "a parent context can read every descendant's app
  state" is ambient authority by any other name if it applies to all state
  implicitly. Todo becomes the first opt-in app, not a special case in the host.
- **Read-only, always.** The parent view can display and filter; it cannot write
  through to a child's file. A write-through model needs conflict rules,
  ownership rules, and an undo story, and buys little: the user can zoom into the
  child context to edit, which is the motion the portal model already teaches.
- **Ancestry is the context tree, not the directory tree.** A context rolls up
  its *sub-contexts*, which are already an explicit parent/child relation in the
  router with a shipped UI (portals, zoom, the depth stack). Using filesystem
  ancestry instead means any context rooted anywhere under the home directory
  silently starts feeding the home context — an accidental, invisible data flow
  the user never asked for, across projects that have nothing to do with each
  other. This is the one place the recommendation diverges from Ian's framing
  ("a parent *directory's* context"), and it is the second open decision in §5.
- **Depth is bounded by the tree, and the whole subtree is included.** With
  ancestry defined as the context tree, depth is already user-authored — nobody
  accidentally has a twelve-deep context tree — so no arbitrary cap is needed.
  A cap on a filesystem-ancestry model would be mandatory, which is itself an
  argument for the tree.
- **Child state stays independently addressable.** Rollup adds a view; it moves
  nothing. The child context keeps its own root, its own state file, and its own
  panes, and remains fully usable with the parent closed. That is what makes
  rollup reversible — turning it off can never lose data.

### Rejected

- **Merge child state into the parent's file.** Destroys the property that state
  lives with the directory (commandment 1: portable files that still make sense
  without Plexi), and makes rollup irreversible.
- **General implicit aggregation of all context-scoped state.** Every app that
  ever declares `context` scope would silently gain cross-context reads it never
  asked for, and no app author could reason about who sees their bytes.
- **Rollup by filesystem ancestry with a depth cap.** Rejected on the accidental
  data-flow argument above; retained as an option for Ian in §5 because it is
  the literal reading of his ruling.
- **A new "rollup" pane type.** The portal already renders a child context
  inside a parent. Rollup is a data question, not a layout one.

## 4. Check against NORTH_STAR

**Supported by the direction:**

- *"Two sources of truth for state or permissions"* is listed under what does not
  belong in Plexi. Two contexts on one root is precisely two sources of truth for
  one address — the hard stop deletes a named anti-goal.
- *Commandment 1 — all data lives in portable open files.* Both designs keep
  state in the directory it belongs to; rollup reads, never relocates.
- *Commandment 4 — every feature reachable through the CLI.* The guard's
  resolution moves (`switch`, `set-root`, `--steal`, `doctor`) are all CLI-first
  and agent-drivable.

**Tensions, stated plainly:**

- *Commandment 2 — friction is a bug.* A hard stop is friction by design. It is
  justified only because the alternative is silent data loss, and only if the
  error names the next move. An error that just says "already in use" would
  fail this commandment.
- *Commandment 10 — apps never get ambient authority.* Rollup is a
  cross-context read. The opt-in declaration is what keeps it inside the
  permission model; an implicit version would violate this commandment outright.
- *"Grown, not universal."* Rollup shaped as "todos specifically" is a
  special case in the host, which the direction resists. The declared-capability
  shape is the general version of the same feature, which is why it is
  recommended over hard-coding todo.

Neither design conflicts with the fractal/spatial direction: contexts stay
nestable, and rollup follows the same parent/child relation the spatial model
already uses — which is the strongest argument for tree ancestry over
filesystem ancestry.

## 5. Open decisions — Ian's to own

1. **Does the hard stop apply retroactively?** Recommendation: no — load, flag,
   and offer one command. Alternatives: refuse to load, or auto-migrate.
2. **Are sub-contexts exempt?** They inherit the parent's root by construction
   today, so the rule as stated forbids them. Recommendation: exempt them and
   restate the rule as "no two *top-level* contexts share a root." The
   alternative is giving sub-contexts their own distinct roots, which changes
   what a sub-context is and is a much larger change than this brief covers.
3. **Rollup ancestry: context tree or filesystem?** Recommendation: the context
   tree. Ian's ruling as stated says directory ancestry. This is the one place
   the recommendation departs from the ruling, and it is worth an explicit call.
4. **Is rollup todo-specific or general?** Recommendation: general mechanism,
   opt-in per app, todo as the first adopter.
5. **Does a child's state stay independently addressable once rolled up?**
   Recommendation: yes, unconditionally — it is what makes the feature
   reversible.
6. **Is rollup read-only?** Recommendation: yes for v1. Write-through can follow
   once the read view has been lived with.
7. **Does a sub-context follow its parent's `root` or its `path`?** Today the
   keyboard path takes `path`, which is frozen at creation and goes stale on
   the first set-root. Recommendation: sub-context creation reads `root` only,
   and `path` keeps its narrow meaning as the creation-time working directory.
   The alternative — keeping the two fields in sync — preserves a second source
   of truth for the same question, which is the anti-goal §4 cites.

## 6. Stints that would follow approval

Described, not filed — this task files none.

- **The guard.** `Context::root` made private to the router behind fallible
  `set_root` / `clear_root`, every assignment and clear path routed through
  them, the path-comparison rule, and the rejection error. Includes the
  sub-context exemption as ruled, and the `path`-vs-`root` decision from §5.7.
- **Duplicate detection at load + `plexi context doctor`.** Warn trace, sidebar
  marking, and the one command that lists conflicts and resolves them.
- **`plexi context set-root --steal`.** The takeover move the error text
  promises.
- **Rollup: the declaration and the host read API.** Manifest opt-in, attributed
  aggregate read across a context's subtree, capability-gated.
- **Rollup: the todo adoption.** Todo declares the capability and renders the
  attributed parent view. Sequenced after the todo rebuild (0674) rather than
  against the current app.

The state-loss defects the audit found are already filed as their own stints and
are **not** blocked on this ruling; they are fixes to shipped behavior, while
this brief is a change to the model.
