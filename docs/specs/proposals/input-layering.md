# Input Layering Contract

**Status:** Proposal (extracted from PR #245 during the 2026-04-15 alpha consolidation)
**Last updated:** 2026-04-15
**Owner:** plexi-core
**Promotion target:** Folds into `releases/plexi-v2.0.md` §7 (capability enforcement) as §7.5 once the implementation lands.

---

## TL;DR

v1 has no centralized keyboard routing. Overlays, panes, modals, and apps all call `ui.input_mut(|i| i.consume_key(...))` independently, and the outcome of any keypress is determined by widget rendering order plus ad-hoc guards. This is how the command palette can be "open" but still leak Enter/arrows to the underlying app (alpha-bug #240), how quick-note cursor activation on pane navigation breaks (alpha-bug #236), and how egui TextEdit can eat app-level shortcuts like Cmd+S before the intended handler runs. Every new overlay re-discovers the same bug class.

v2 makes routing explicit and centralized via a host-owned **input layer stack** with named, priority-ordered layers.

## Design

```rust
pub trait InputLayer {
    fn name(&self) -> &'static str;
    fn handle(&mut self, ev: &InputEvent, ctx: &mut InputContext) -> InputDecision;
}

pub enum InputDecision { Consumed, Passthrough }

pub struct InputLayerStack {
    layers: Vec<Box<dyn InputLayer>>, // top of stack = highest priority
}
```

Default priority (top to bottom):

1. `Overlay::CommandPalette` — when active
2. `Overlay::QuickNote` — when active
3. `Overlay::NotificationPalette` — when active
4. `Overlay::AgentMode` — per-pane, when active
5. `Pane::Focused` — currently focused pane's input owner
6. `Pane::Unfocused` — background panes (input mostly dropped)

Each frame: drain egui's input queue once, walk the stack top-down, first `Consumed` wins. No egui widget rendering occurs until after the walk — `TextEdit` widgets never see events a higher-priority layer claimed.

## Rules

1. **All input routing goes through the stack.** No widget-level `consume_key` calls in overlay code.
2. **Layers push on activation, pop on dismissal.** Stack owns lifetime; layers never self-remove mid-frame.
3. **Overlays at the same priority level are mutually exclusive.** Opening a second top-level overlay dismisses the first.
4. **`Pane::Focused` is the default layer for external apps.** Subprocess receives a `Key` event on stdin only if every higher layer declined.
5. **The stack is observable.** `EventKind::InputLayerChanged { layer, active }` fires on the event bus (`releases/plexi-v2.0.md` §4) on every push/pop.
6. **Capability enforcement composes with layering.** A pane's `observes` capability still gates event bus subscriptions. Input layering governs raw event routing; capabilities govern structured event observation.

## What this fixes

| Bug / class | Today | Under the layer stack |
|---|---|---|
| **#240** Command palette leaks input | TextEdit or lower panes consume before palette can guard | `Overlay::CommandPalette` is top of stack; pane layers never see the event |
| **#236** Quick note cursor doesn't activate on pane nav | Pane focus moves but inner TextEdit doesn't re-claim egui focus | Navigation rebuilds `Pane::Focused` layer with the destination's input owner |
| **`consume_key` modifier exactness** | `consume_key(NONE, Enter)` matches Enter+Shift | Layers receive raw events and decide for themselves |
| **TextEdit eats Cmd+S** | Widget render consumes before app handler | App-level shortcuts live in a pane-level layer that runs before widgets render |
| **Escape ownership** (already resolved in v2.0) | Host consumed Escape before app saw it | Host's Escape handler is just another `InputLayer` that can be reordered |

## SDK surface

**None.** Host-internal. External apps see the same `Key` events on stdin, just fewer spurious ones — the ones they get are guaranteed to be events no higher-priority layer wanted.

## Implementation location

New file: `src/input_layer.rs` — stack, trait, default priority order, bus event emission.

Refactored consumers:
- `src/keys.rs` — thin draining function that walks the stack per frame.
- `src/command_palette.rs` — implements `InputLayer`.
- `src/quick_note_app.rs` — implements `InputLayer` for the overlay path.
- `src/notification_palette.rs` — implements `InputLayer`.
- `src/agent_mode.rs` — implements `InputLayer` per-pane.
- `src/pane_ops.rs` — pane focus changes drive `Pane::Focused` layer rebuilds.

## Testing story

The event bus makes this testable without UI automation:
1. Push an `Overlay::CommandPalette` layer.
2. Inject a synthetic `Key { key: "Enter", .. }` into the input queue.
3. Drain the stack.
4. Assert the CommandPalette layer's handler consumed the event.
5. Assert the focused pane's external-app subprocess received zero `Key` events on its stdin.
6. Assert exactly one `InputLayerChanged` event was emitted on the bus.

First time input routing becomes deterministically testable in Plexi.

## Closes

- alpha-bug #240 (command palette focus leak)
- alpha-bug #236 (quick-note cursor activation on pane nav)
- The CLAUDE.md `consume_key` exactness lesson class

## Promotion plan

When `src/input_layer.rs` ships with at least `CommandPalette` and `Pane::Focused` layers migrated:
1. Inline this content into `releases/plexi-v2.0.md` as §7.5 (Input Layering Contract).
2. Add `InputLayerChanged` to the event bus event kinds in `releases/plexi-v2.0.md` §4.
3. File a tracking issue against #224 v2.0 umbrella with the implementation checklist.
4. Delete this file.
