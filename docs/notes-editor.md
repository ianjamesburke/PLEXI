# Plexi Notes Editor

Status: active

Stint: 0472, 0473, 0317, 0474, 0475, 0476, 0477, 0478, 0318, 0479

## Destination

Plexi Notes is a keyboard-first Markdown editor whose ordinary editing behavior is dependable enough to replace Obsidian for daily scratch notes. The editing engine is Plexi-native and reusable by future file and code editors. Notes adds Markdown-aware presentation, links, attachments, and persistence around that shared core rather than owning a second text-editing implementation.

The editor has one document model, one selection model, and one undo history. Inserting text, replacing a selection, indentation, list continuation, deletion, paste, IME composition, drag-and-drop, and Markdown commands all enter through the same transaction boundary. Rendering never becomes the authority for document contents or cursor positions.

## Architecture

Upgrade the host UI stack to egui 0.34 before extracting the editor core. Use Ferrite's MIT-licensed editor as the pinned reference implementation for rope-backed editing, selections, history, IME, scrolling, and layout. Port only the cohesive editing subset Plexi needs, retain attribution and provenance, and wrap it behind Plexi-owned document, command, view, and semantic-state interfaces. Do not import Ferrite's application shell or configuration model.

The shared core owns text storage, byte/grapheme-safe cursor positions, selections, edit transactions, history, keyboard movement, viewport layout, and input-method state. Notes owns Markdown commands, preview decoration, link behavior, attachment storage, autosave, and note metadata. A future code mode adds syntax-oriented presentation and commands to the same core; it does not swap in a second editing widget.

Per-line `TextEdit` widgets cannot satisfy this contract because selection, undo, IME composition, and cursor navigation must cross rendered Markdown blocks coherently. `egui_code_editor` is useful as a source of highlighting ideas but its `TextEdit`-wrapper architecture is not the editing foundation. A browser-based editor is outside this design because the host needs a native egui surface, deterministic scene testing, and a shared input path with other Plexi panes.

## Editing Contract

Normal platform conventions apply to selection, word and line movement, deletion, clipboard operations, undo and redo, Home and End, Page Up and Page Down, and modifier-click. Text and selection remain correct across ASCII, Unicode grapheme clusters, emoji, combining characters, and IME composition.

Tab and Shift-Tab indent and outdent the current line or every selected line as one undoable transaction. Enter continues ordered lists, unordered lists, block quotes, and task lists; Enter on an empty continuation removes the marker. Smart Backspace removes one logical indentation level without corrupting adjacent text. Find, replace, scroll-to-caret, click placement, drag selection, autosave, and reopening preserve the same document semantics.

## Markdown Experience

Notes provides source mode and Live Preview. In Live Preview, Markdown source syntax is revealed around the active editing block while inactive blocks render their styled representation. Movement and selection across those blocks remain continuous because the rendered view maps back to positions in the single source document. The preview supports the Markdown structures already recognized by Plexi's Markdown renderer and degrades unknown syntax to editable source.

Markdown links and wiki-style note links are editable as source near the caret and presented readably elsewhere. Activating an external URL requires an explicit modifier gesture and uses the host URL-opening capability. Activating an internal note link opens or focuses that note without turning ordinary cursor clicks into navigation.

## Attachments

Dropping a supported local image onto the hovered Notes editing surface copies it into an `assets/` directory beside the note using a collision-safe stable name and inserts a relative Markdown image reference at the drop position. A note in a storage tier therefore keeps its attachments inside that tier, so a project's notes and their images move with the project; tier addressing itself is `src/host/AGENTS.md`'s contract. Supported local formats are the formats decoded by the host image stack, including PNG, JPEG, GIF, WebP, and BMP. A dropped image URL inserts a remote Markdown image reference without downloading implicitly.

Inline images are bounded to the editor width, cached, and represented by a visible placeholder when loading or decoding fails. A production drop is delivered exactly once to the hovered pane. The same handler is driven by real host drop events, a pane CLI command, and TOML test scenes.

## Agent Validation Contract

Editor behavior is expressible as deterministic command sequences against the pure editor core and as TOML host scenes. Semantic pane state exposes document text, selections, cursor positions, undo state, active Markdown block, visible links and images, scroll state, dirty state, and last save result without making test-only state authoritative.

Installed PR builds can boot a channel-scoped host, open Notes, focus its editor, send text and key commands, perform file or URL drops, wait on semantic conditions, inspect the saved note and copied attachment, and capture the pane through `plexi host screenshot`. Failure bundles include the scene, event trace, semantic pane state, logs, saved note, attachment manifest, and screenshot.

The release gate exercises editing, Unicode and IME, selection, history, Markdown transactions, Live Preview, links, image drops, persistence, and stress sequences through the real installed host in addition to unit and harness coverage.

## Non-Goals

The Notes overhaul does not include LSP integration, collaborative editing, arbitrary HTML execution, a plugin API, rich-text storage, or replacing Markdown as the on-disk source of truth. Multi-cursor editing may be retained when it falls naturally out of the extracted core, but it is not a release requirement.

## Provenance

The extracted editor behavior must record the upstream Ferrite repository and pinned commit, preserve its MIT license and notices, and document meaningful divergences in the owning code module. Dependency versions and provenance live with the dependency or source module rather than being duplicated here.
