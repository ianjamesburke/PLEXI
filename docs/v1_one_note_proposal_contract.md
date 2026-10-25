# V1 One-Note Proposal Authority and Handoff Contract

Stint: 1009  
Status: active  

## 1. Summary and Scope Seams

This contract defines the smallest safe specification for a manually invoked, one-selected-note proposal exemplar in Plexi V1. It resolves the exact CLI/host/package seam, input/output schemas, provider disclosure, user decision handoff, and the authority boundary between Plexi's host broker and harness-native tools.

This is a **proposal contract for downstream implementation stints**, not a claim that the complete workflow is already wired. Today, `plexi note list`, the note tier predicate in `src/notes.rs`, and its validation helpers exist. The `plexi note review <note-id-or-path>` subcommand, `review:*` slots, staging lifecycle, and UI disclosure modal described below are proposed seams for stints 0796, 1014, and 1015.

### 1.1 Architectural Invariants
- **Manual Invocation Only:** V1 supports only explicit human triggering of a single selected note review. Automated routines, background daemons, and scheduled loops are deferred to downstream work (stint 1015).
- **Single Canonical Note Selection:** The review workflow operates on exactly one note in an existing context or global tier. Bulk crawling, multi-note scanning, and cross-project aggregation are strictly excluded.
- **Existing Scope Tiers:** Storage remains anchored in Plexi's two-tier user data model (`StateScope` / `UserDataKind::Notes` in `src/host/state_scope.rs`) — a channel-neutral global tier (`~/.plexi/notes/`) and per-context tiers (`<context_root>/.plexi/notes/`). No new "personal home" or contextless agent runtime is introduced.
- **Untrusted Input Boundary:** Markdown note content is treated as untrusted data, never as workflow instructions or authorization grants. Embedded directives, command patterns, markdown links, or external URLs inside note text cannot widen the agent's authority or trigger host actions.
- **Strict Source Preservation:** In V1, the source note and its attachments (`.plexi/notes/assets/`) are immutable during review. User decision options (`keep`, `defer`, `copy-proposal`) never mutate, overwrite, archive, or delete the original note. Approved in-place mutations and transactional checkpointing are owned downstream by stint 1014.

### 1.2 Downstream Stint Handoff
- **Stint 1014 (Approved Note Apply and Durable Checkpoint):** Owns transactional in-place mutation, atomic write-backs, hash-guarded apply, destination resolution, and undo logs.
- **Stint 1015 (Scheduled One-Note Wrapper Qualification):** Owns periodic routine scheduling, change-detection pre-flight checks, missed-run handling, and idle/hidden-host execution semantics.
- **Deferred Post-V1 Capabilities:** Hourly routines, contextless agents, global directory crawling, and Jev semantic ranking remain out of scope for this V1 exemplar.

---

## 2. Exact Commands and Interfaces

### 2.1 Note Selection Seam
Plexi enumerates available notes via `notes_list_cli` (`plexi note list`), which calls `cli_notes_scopes()` to resolve the anchored context tier (derived from cwd) and the global tier. `notes_list_cli` prints paths sorted newest-mtime first.

`notes_list_cli` is an enumeration for human selection, **not** an automated agent queue or bulk JSON feed. The human user (or host UI) selects exactly one canonical path:
```bash
plexi note review <path>
```
In the GUI, the user selects a note row in the Notes Picker (`src/overlays/notes_picker.rs`) or clicks "Review Note" in the note editor chrome, dispatching a host action targeting the canonical note path.

### 2.2 Intake Validation and Bounded Handoff
Before staging or contacting any provider, the host executes bounded intake validation (`validate_note_bounds` in `scripts/probe-one-note-authority.py`, and `tier_root_for_note` in `src/notes.rs`):

1. **Tier Membership:** The path must satisfy `tier_root_for_note(path).is_some()`. Files outside known note tiers or under attachment directories (`.plexi/notes/assets/*`) are rejected with `PathOutsideTier`.
2. **Size Cap:** Input size is capped at 64 KiB (65,536 bytes) UTF-8 text. Notes exceeding 64 KiB are rejected with `Oversized` before any staging or network I/O occurs.
3. **Encoding Validation:** The note bytes must be valid UTF-8. Binary content or files containing null bytes are rejected with `InvalidEncoding`.
4. **Saved vs. Dirty State:** Review operates strictly on saved disk bytes. Unsaved editor buffers in the UI are ignored; the user must save (`Cmd+S`) before invoking review.
5. **Content Hash Fingerprint:** The host computes the SHA-256 digest of the saved note content on disk (`source_hash`).
6. **Attachment and URL Policy:** Referenced attachments (e.g. `assets/diagram.png`) and web URLs (`https://...`) embedded in note text are treated strictly as plain inert text. Neither the host nor the worker resolves, reads, or fetches referenced external resources.

### 2.3 Staging Architecture and Retention
To isolate the worker from ambient workspace files, the host stages approved input into a dedicated ephemeral path:
```
<runtime_staging_dir>/proposals/<source_hash>/input.md
```
- **Permissions:** The staged file is marked read-only (`0400`).
- **Retention & Cleanup:** The ephemeral staging directory is purged upon user decision (`keep`, `defer`, `copy-proposal`), worker timeout, or cancellation.
- **Audit Logging:** Audit records retain only metadata (canonical path, SHA-256 hash, byte size, provider name, and final decision). Raw note text and secret credentials are never retained in audit records.

---

## 3. Schemas

### 3.1 One-Note Input Schema (JSON)
The host constructs a bounded input descriptor passed to the worker:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "NoteReviewInput",
  "type": "object",
  "required": [
    "canonical_path",
    "source_tier",
    "source_hash",
    "byte_size",
    "context_id"
  ],
  "properties": {
    "canonical_path": {
      "type": "string",
      "description": "Absolute canonical filesystem path of the source note"
    },
    "source_tier": {
      "type": "string",
      "enum": ["global", "context"],
      "description": "Storage tier owning the note"
    },
    "source_hash": {
      "type": "string",
      "pattern": "^[a-f0-9]{64}$",
      "description": "SHA-256 digest of the saved note bytes at intake"
    },
    "byte_size": {
      "type": "integer",
      "maximum": 65536,
      "description": "Size of note in bytes, capped at 64 KiB"
    },
    "title": {
      "type": ["string", "null"],
      "description": "Extracted frontmatter title or first heading, if present"
    },
    "context_id": {
      "type": "integer",
      "description": "Owning context ID"
    }
  },
  "additionalProperties": false
}
```

### 3.2 Proposal Output Schema (JSON)
The worker outputs a structured review proposal:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "NoteProposal",
  "type": "object",
  "required": [
    "schema_version",
    "source_path",
    "source_hash",
    "summary",
    "proposed_body",
    "suggested_action",
    "provider_info"
  ],
  "properties": {
    "schema_version": {
      "type": "string",
      "const": "1.0"
    },
    "source_path": {
      "type": "string",
      "description": "Canonical path of reviewed source note"
    },
    "source_hash": {
      "type": "string",
      "pattern": "^[a-f0-9]{64}$",
      "description": "SHA-256 hash matching the intake note input"
    },
    "title": {
      "type": ["string", "null"],
      "description": "Proposed refined note title"
    },
    "summary": {
      "type": "string",
      "description": "Concise summary of review findings and proposed changes"
    },
    "proposed_body": {
      "type": "string",
      "description": "Complete refined markdown body text"
    },
    "suggested_action": {
      "type": "string",
      "enum": ["keep", "refine", "split", "archive_candidate"],
      "description": "Advisory recommendation for the human user"
    },
    "suggested_destination": {
      "type": ["string", "null"],
      "description": "Optional recommended destination tier or category"
    },
    "provider_info": {
      "type": "object",
      "required": ["provider", "model", "timestamp_utc"],
      "properties": {
        "provider": { "type": "string" },
        "model": { "type": "string" },
        "timestamp_utc": { "type": "string", "format": "date-time" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
```

---

## 4. Provider Disclosure and Transmission Point

### 4.1 Explicit Disclosure Dialog
Before transmitting note text to any provider (local or remote), the host halts and displays an explicit disclosure prompt:
- **Target Provider:** Provider name (e.g. Anthropic, OpenAI, OpenRouter, Local Ollama).
- **Model Identifier:** Configured model name.
- **Selected File:** Path and byte size of the staged note.
- **Egress Disclosure:** Notice that data will leave the local device when a remote provider is selected.

User confirmation is required. Without confirmation, transmission is aborted and staging is purged.

### 4.2 Retry Policy
If transmission fails due to transient network failure, retrying requires active user consent or a session-bound grant. The disclosure prompt is never bypassed on retry.

### 4.3 Zero Extra Data Guarantee
The transmission payload contains strictly:
1. Fixed system prompt instructions for note review.
2. The staged note markdown content.

No directory listings, neighboring context notes, workspace filenames, environment variables, or secret keys are transmitted.

---

## 5. Parent-Visible Status and Result

The worker's lifecycle is exposed to the parent context through Plexi pane slots (`status`, `result_path`, `last_error`):

| Slot Status | Meaning |
|---|---|
| `review:staged` | Intake validation passed, staged input prepared. |
| `review:in_flight` | Worker is executing review inference. |
| `review:proposed` | Proposal generated and validated against schema; awaiting user decision. |
| `review:stale` | Source note was edited on disk during generation; proposal invalidated. |
| `review:failed` | Intake, inference, or schema validation failed; details in `last_error`. |

The parent pane or host UI monitors progress using:
```bash
plexi pane slot read status --pane-id $WORKER_PANE_ID
```
Or by reading the structured response file written upon completion.

---

## 6. User Decisions and Source Preservation Guarantee

When a proposal is presented, the user has three mutually exclusive choices:

1. **`keep`:**
   - User decides the original note needs no changes.
   - Proposal is dismissed and staging cleaned up.
   - Original note and attachments remain untouched.
2. **`defer`:**
   - User postpones deciding.
   - Proposal record is preserved in `.plexi/staging/proposals/` for later inspection.
   - Original note and attachments remain untouched.
3. **`copy-proposal`:**
   - User accepts the proposal content.
   - Host copies `proposed_body` to the system clipboard or creates a new distinct note (e.g. `<stem> (Proposal).md`).
   - Original note and attachments remain untouched.

**V1 Preservation Rule:** Neither the CLI nor the host ever overwrites, truncates, renames, archives, or deletes the source note in this workflow.

---

## 7. Repeat and Restart Handling

- **Repeated Invocation with Identical Hash:** If review is requested on a note whose `source_hash` matches a cached proposal, the existing proposal is displayed immediately without redundant provider inference.
- **Source Edits During Generation:** If the source note's mtime or SHA-256 digest changes while review is in-flight, the resulting proposal is marked `review:stale`. A stale proposal cannot be copied as an active recommendation; the user must trigger a fresh review against the updated content hash.

---

## 8. Authority Boundary, Harness Qualification, and Release Gate

### 8.1 Host Broker vs. Harness-Native Tools
Plexi's Permission Monitor (`src/broker/gate.rs`) intercepts host-brokered capabilities: `secret.get`, `secret.list`, `pane.send`, and `app call`. Stint 0964 qualified that agent `[permissions]` posture reliably enforces `deny` and `ask` for these host-brokered tools (`docs/security/worker-permission-posture-qualification.md`).

However, **harness-native tools** (e.g. bash execution, file read/write tools built into `pi`, `claude -p`, `codex`, `agy`) execute inside the child harness process on the host OS, completely bypassing Plexi's host broker gate. A system prompt saying "You are read-only" provides zero security enforcement against a compromised or hallucinating model.

### 8.2 Supported Harness Configuration
The qualified installed harness is `pi 1.4.0`. The only live V1 configuration is:

```text
pi --print --no-tools --no-session --no-context-files --no-approve --offline
```

The host validates and hashes the one saved note, builds the bounded proposal payload, and sends it to the worker's stdin. `--no-tools` disables `read`, `bash`, `edit`, `write`, and all other worker tools; it does not make a staged file selectively readable. There is no `--isolated-dir` flag in this Pi version. The probe verifies `pi --version`, `pi --help`, the real launch argv, the captured stdin digest/size, and that the outbound capture contains no unselected note, attachment, directory-listing, or secret bytes.

The native tool-bearing negative control uses `pi --print --tools read,write,bash`. Pi's native tools retain the OS user's filesystem authority and cannot enforce a single-note boundary. Its result is `FAIL_UNRESTRICTED` and `WITHHELD_SYNTHETIC_FIXTURE_ONLY`; it is not live V1 acceptance. Prompt wording, a working-directory convention, and an invented isolation flag are not security boundaries.

### 8.3 Release Decision Gate
If the selected harness cannot enforce the path boundary (i.e. cannot demonstrably deny writes to the source note and reads of unrelated files), **live V1 acceptance of the exemplar MUST BE WITHHELD**.

In that event, the exemplar is restricted to a **synthetic-fixture-only demonstration** until OS-level sandboxing (e.g. macOS Seatbelt / Linux bubblewrap) or harness native tool restrictions are qualified. Prompt-only isolation is explicitly rejected as a security boundary.

---

## 9. Authority Probe and Negative-Control Qualification Evidence

The contract includes an automated, fixture-backed authority probe (`scripts/probe-one-note-authority.py`) and test suite (`scripts/test_probe_one_note_authority.py`).

### 9.1 Qualification Vectors
1. `pi --no-tools` is verified against the installed binary and help output; the worker has zero file/bash tools.
2. The exact argv and stdin payload are captured; only the selected note's bounded content and schema framing are present.
3. The fixture includes an unselected sibling, global note, attachment, directory-listing candidate, and secret environment value; byte-for-byte inspection proves they are absent.
4. The native `--tools read,write,bash` mode is a negative control: because it can reach unrelated paths and write the source, it records `FAIL_UNRESTRICTED` and withholds live acceptance.

### 9.2 Negative-Control Proof
The probe includes an explicit native-tool negative control using the real installed Pi flags. It proves that tool-bearing execution is unrestricted and records `FAIL_UNRESTRICTED` / `WITHHELD_SYNTHETIC_FIXTURE_ONLY`, while the accepted path is the real `--no-tools` stdin-only subprocess.

### 9.3 Retained Evidence Record
The retained qualification evidence is saved at `docs/security/probe-one-note-authority-evidence.json`. It records:
- Schema version, harness path/version, exact launch flags, and tool-mode decision.
- Source content hash and byte size; raw note, sibling notes, attachment bytes, directory listings, and secret values are not retained.
- Payload capture verdict (`ONLY_SELECTED_NOTE`) and overall `VERIFIED_RESTRICTED` result.
