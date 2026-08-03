Stint: 0728
Status: active

# The search → results → detail pattern

Search for a thing, get a list of results, pick one, view it. This is not a
Wikipedia feature — it's the shape of most apps anyone will build on Plexi.
There is currently no sanctioned pattern for it, so apps hand-roll it, badly.
`apps/wikipedia/wikipedia.py` is the evidence case this doc uses throughout;
`apps/github-issues/main.py` and `apps/kraken/main.py` show the same two
failures independently, confirming this is a cross-cutting SDK gap, not a
one-off mistake.

This doc is a decision, not a diff. It names two decisions Ian must make
(§4, §5) and proposes a default answer for each so the implementation stint
can start the moment he picks.

## 1. The two failures, in wikipedia.py today

**Results are raw `Button`s, selection is a style hack, keyboard nav is
hand-rolled** (`wikipedia.py:400-408`, selection dispatch `wikipedia.py:148-159`):

```python
rows = [
    Button(title, f"wiki-result-{index}",
           style="primary" if index == data["selected"] else "secondary")
    for index, title in enumerate(data["results"])
]
```

`SelectList` (`sdk/python/plexi_sdk/ui.py:2085`) already does everything this
block is faking: keyboard nav (`handle_key`, j/k/arrows), scroll-into-view,
click hit-testing, and a real selected-row visual. `AUTHORING.md:233` already
says *"List + detail navigation: use `SelectList`... Never reimplement this
by hand."* Wikipedia breaks a rule that's already written down and already
followed by five other exemplar apps (github-issues, kraken, permissions,
stats, todo). This is a discoverability/enforcement failure, not a missing
primitive — see §4.

**Pending state is a hand-threaded boolean, forked at three independent call
sites:**

```python
# _search_view  (wikipedia.py:359-362)
Card([Skeleton(rows=3)]) if data["loading"] else Card([...form...])

# _article_view (wikipedia.py:428-430)
Card([Skeleton(rows=5)]) if data["loading"] else Card([Text(...)])

# _status       (wikipedia.py:445-446)
if data["loading"]: return "Loading"
```

Every region that can be pending re-derives it from the same global
`data["loading"]`. Nothing in the SDK lets an app say "this region is
pending" once. `github-issues/main.py:286,343,351` and `kraken/main.py`
(6+ sites) do the identical fork. This is the real SDK gap — see §5.

## 2. What already exists (and is under-used)

| Primitive | File | What it does |
|---|---|---|
| `SelectList` | `ui.py:2085` | Stateful list: `handle_key()`, `hit_index()`, scroll, real selection |
| `Skeleton(rows=N)` | `ui.py:908` | Static placeholder rows (no shimmer — just empty badges) |
| `Spinner` | `ui.py:846` | Thin wrapper over `{"type": "spinner"}` |
| `EmptyState` | `ui.py:~890` | No-results affordance |
| `loading_pill` | `ui.py:1773` | Canvas-mode-only stale-while-revalidate: keep old content, overlay a pill |

One real trap found during research, worth fixing alongside this stint:
`ListRow`/`LeadingIcon`/`RowChip` (`ui.py:2607-2687`) are a **second, unrelated
`ListRow` family** — canvas-mode row descriptors for `ctx.list_view(...)`,
not `to_node()` components. They share a name with `SelectList`'s declarative
item vocabulary (`{name, description, leading, trailing}`) but are a
different code path. Grep found zero app usages of the canvas
`ctx.list_view` path today — nothing depends on keeping them. Per
CLAUDE.md's "no parallel vocabulary" rule: **delete them** in the
implementation stint. `loading_pill` should be re-scoped to
"canvas-mode only" in its docstring, since `Pending` (§5) is the tree-mode
answer.

Also found: `PlexiEvent::ListSelect`/`ListActivate` exist in the wire schema
(`src/protocol/events.rs:462-469`) but nothing in `src/` constructs them
outside serde round-trip tests — the host does not drive list keyboard nav
today. Every `SelectList` app still calls `handle_key()` from its own
`on_key`. Don't design against host-native list events existing; they aren't
shipped.

## 3. Mockup: before / after

**Before** (today, 3-screen full swap, loading forks 3 places):

```
┌ Search ──────────┐   ┌ Results ─────────┐   ┌ Article ─────────┐
│ [search box]      │──▶│ [Button] Detroit  │──▶│ ░░░░░░░░░░░░░░░  │
│ [Search Wikipedia]│   │ [Button] Detroit  │   │ ░░░░░░░░░░░░░░░  │  loading
│                    │   │        River      │   │ ░░░░░░░░░░░░░░░  │
└────────────────────┘   └───────────────────┘   └───────────────────┘
        ░░░ skeleton            no keyboard            skeleton on
        while searching         nav (hand-rolled          every reopen,
                                 j/k in update())          even cached articles
```

**After** (proposed, results list persists, detail is an independent
pending region — see §6 for why this is the recommended shape, not just a
mockup choice):

```
┌ Wikipedia ────────────────────────────────────────────────┐
│ [search box: "Detroit"]                                     │
├───────────────────┬──────────────────────────────────────┤
│ ▸ Detroit          │  Detroit                                │
│   Detroit River     │  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │
│   Detroit (film)    │  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │  ← only this
│   Detroit Pistons   │  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │    region
│                     │                                          │    pending
│  SelectList:        │  Pending(active=data["loading"],        │
│  j/k moves ▸,        │          child=Text(data["article"]))   │
│  results stay        │                                          │
│  mounted             │                                          │
└───────────────────┴──────────────────────────────────────┘
```

Selecting a new result no longer blanks the whole screen — the list stays
interactive and highlighted while only the detail pane shows its skeleton.
This is the "partially loaded" state the stint brief asks for explicitly.

## 4. Decision: results-list vocabulary

**Recommendation: no new SDK node. `SelectList` already covers this; the
gap is enforcement, not primitives.**

- `SelectList`'s item shape (`name`, `description`, `leading`, `trailing`)
  maps directly onto a search result (title + snippet).
- Keyboard nav, scroll, and hit-testing already exist on it.
- `AUTHORING.md` already prescribes it; wikipedia is the outlier, not the
  rule.

What actually needs to happen: wikipedia gets rewritten to use it (implementation
stint), and the canvas-mode `ListRow` family gets deleted so "ListRow" only
ever means one thing (§2). No sweeping SDK addition needed for this half of
the task — the sweep is in §5.

*Alternative considered and rejected:* a first-class `SearchResultsList`
node. Rejected because it would duplicate `SelectList`'s state machine
(selection, scroll, key handling) for no behavioral gain — the only thing
missing is that apps aren't calling the primitive that already exists.

## 5. Decision: pending-state model

This is the real sweeping change and where Ian's taste call matters most.

**Recommendation: an SDK-level `Pending` wrapper component, not a
host-schema `pending` property, for this stint.**

```python
Pending(
    active=data["loading"],
    child=Text(data["article"]),
    placeholder=Skeleton(rows=5),   # optional, see open question below
)
```

`to_node()` returns `placeholder.to_node()` when `active`, else
`child.to_node()`. One call site per pending region, zero host changes,
100% declarative — the app still only describes structure, per NORTH_STAR's
"apps declare structure; the host renders."

*Alternative considered:* a true host-owned `pending` node/property, where
the host infers the skeleton shape from the child's own layout and animates
it (shimmer). This is the more architecturally pure long-term answer, but it
needs new WIT schema (`wit/plexi.wit`), a new decode arm in
`wasm_python.rs`, and host-side shimmer rendering that doesn't exist today
(`Skeleton` is currently static empty badges, no animation). That's
implementation scope well beyond an M design task, and nothing here proves
it's needed yet — `Pending` solves the stated problem ("no app should ever
thread one boolean through three render functions again") without it.
Recommend treating it as a **separate future stint**, informed by how far
`Pending` gets apps once it ships, not gated on this one.

## 6. Decision: where the loading affordance lives

**Recommendation: per-region**, not per-row or per-pane.

- Per-pane (today's wikipedia behavior) is too coarse — it blanks
  already-loaded content (the results list) every time an unrelated fetch
  starts.
- Per-row only earns its complexity when individual rows stream in
  independently. Wikipedia's search returns all results atomically, so this
  doesn't apply here — but it's the right answer for, say, a feed that
  paginates.
- Per-region is the shape in §3's "after" mockup: the results `SelectList`
  stays mounted and interactive; only the detail pane — wrapped in its own
  `Pending(...)` — shows a skeleton while its own fetch is in flight.

This also answers "what does a partially-loaded list look like": results
arrived and are fully navigable, article body still pending — exactly the
right half of the screen is skeletal, not the whole app.

## 7. What gets deleted (implementation stint)

- `wikipedia.py`'s raw `Button` results rows and `wiki-result-{index}`
  handler-id parsing (`wikipedia.py:401-408`, `132-139`) — replaced by
  `SelectList`.
- `wikipedia.py`'s inline j/k/up/down dispatch in `update()`
  (`wikipedia.py:148-159`) — replaced by `SelectList.handle_key()`.
- The three independently-forked `if data["loading"]:` branches in
  `_search_view`, `_article_view`, `_status` (`wikipedia.py:361-362,
  428-430, 445-446`) — replaced by `Pending(...)` around only the regions
  that are actually in flight.
- Canvas-mode `ListRow`/`LeadingIcon`/`LeadingAvatar`/`RowChip`
  (`ui.py:2607-2687`) — dead code, zero app usages found, name-collides with
  `SelectList`'s declarative vocabulary. Delete outright.
- `loading_pill`'s docstring gets narrowed to "canvas-mode only" — `Pending`
  is the tree-mode equivalent, so it should stop reading as a general
  pending-state answer.

Follow-on (not this stint, but named so the pattern doesn't stay half
adopted): `github-issues/main.py` and `kraken/main.py` have the identical
`loading`/`pending` boolean fork and should migrate to `Pending` once it
ships.

## 8. Exemplar bar

Rewritten `wikipedia.py`: **≤ 340 lines**, down from 457 today (~25%). Most
of the file's bulk is HTTP/tool-call plumbing (`_handle_tool_call`,
`_fetch_search`, `_quote`, ExposeTools wiring) which is explicitly
out-of-scope (stint 0597) and untouched — the reduction comes entirely from
deleting the hand-rolled list nav (~19 lines), the raw-Button construction
(~15 lines), and collapsing three loading forks into `Pending(...)`
call sites (~2 lines each vs ~6 lines of branching today). The implementation
stint is gated on beating 340, not on hitting an arbitrary round number.

## 9. Open questions for Ian (pick before implementation starts)

1. **Results/detail layout shape** — keep wikipedia's current 3-screen
   sequential swap (search → results → article, least visual change) vs.
   adopt the persistent-list/independent-detail-region split from §3/§6
   (matches other exemplar apps, sets the pattern other search-to-detail
   apps should copy). §6's recommendation assumes the split; if Ian prefers
   the 3-screen swap, `Pending` still collapses the loading forks, but the
   "partially loaded" state described in §6 doesn't apply to wikipedia
   specifically.
2. **`Pending`'s default placeholder** — require an explicit
   `placeholder=Skeleton(rows=N)` every time (more boilerplate, no
   surprises) vs. `Pending` infers a default (e.g. `Skeleton(rows=3)`) when
   omitted (less boilerplate, but a default that doesn't match the eventual
   content's height causes a layout jump).
3. **Canvas `ListRow` family** — delete now (§7, recommended, zero usages
   found) vs. rename to something non-colliding (e.g. `CanvasListRow`) and
   keep as the sanctioned canvas-mode row API for apps doing custom
   `Canvas()` rendering outside the declarative tree.

Once these three are picked, the implementation stint (sibling to 0728) can
land `Pending` in `sdk/python/plexi_sdk/ui.py`, delete the canvas `ListRow`
family per the answer to (3), and rewrite `wikipedia.py` against both.
