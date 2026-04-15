# Plexi Intelligence Protocol

**Status:** Draft  
**Last updated:** 2026-04-11

---

> **Status update (2026-04-11):** This spec is **deferred**. The current architecture has apps managing their own LLM calls directly — apps get API keys from Plexi's directory-scoped secrets manager and call providers themselves. Apps report costs back via `cost_report` events. Plexi logs costs to `costs.jsonl` for per-app, per-directory attribution.
>
> This intelligence proxy protocol becomes relevant when multiple apps need unified model selection and cost caps across them. For now, it serves as a reference design for that future state.
>
> **What shipped instead:**
> - Apps declare required secrets in manifest.toml: `[app.secrets] required = ["ANTHROPIC_API_KEY"]`
> - Apps resolve secrets via `SecretGet` API call (directory walk-up resolution)
> - Apps make their own API calls (requires `network = true` capability)
> - Apps report costs via `cost_report` draw command event
> - Plexi logs costs to `~/.plexi-alpha/costs.jsonl` — per-app, per-directory, per-operation attribution
> - Trust and risk scores use continuous floats (0.0–1.0), not categorical levels

---

## 1. Overview

Plexi provides intelligence (LLM and image generation) as a platform capability rather than letting apps call AI APIs directly. This is a deliberate architectural choice with five concrete benefits:

- **Unified cost tracking.** Every LLM and image generation call across all apps is logged to a single ledger. The user sees total spend, per-app spend, and per-session spend in one place.
- **Spend limits enforced at the platform level.** Apps cannot exceed their budget. Plexi rejects requests before they hit the API when limits are reached — no runaway costs from a misbehaving app.
- **Centralized API key management.** API keys live in the Keychain, managed by Plexi's secrets system. Apps never see or store keys. Swapping providers (e.g., Anthropic → Google) is a config change, not an app change.
- **Model selection abstracted.** Apps request a tier (`low`, `medium`, `high`), not a specific model. Plexi resolves the tier to a model based on system config. Model upgrades are transparent to apps.
- **Full observability.** Every intelligence call is logged with `app_id`, working directory, tier, model, token counts, and cost. This enables auditing, debugging, and cost attribution.

---

## 2. Intelligence Tiers

Apps request intelligence by tier, not by model name. Plexi resolves tiers to specific models based on system configuration.

| Tier | Intent | Example Models |
|---|---|---|
| `low` | Fast, cheap — summaries, classification, simple transforms | Claude Haiku, Gemini Flash |
| `medium` | Balanced — writing, analysis, code generation | Claude Sonnet, Gemini Pro |
| `high` | Maximum quality — complex reasoning, long-form generation | Claude Opus, Gemini Ultra |

Tier-to-model mapping is configured in `~/.plexi-alpha/config.toml` (see Section 8). Apps never specify a model directly. If an app requests a tier that isn't configured, Plexi falls back to `medium`.

An app's manifest can declare a default tier (see Section 3), but individual requests can override it. The manifest default is a hint for cost estimation, not a hard constraint.

---

## 3. New Capability: `intelligence`

### Permission Enum

```rust
// In app_permissions.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligencePermission {
    /// No intelligence access.
    None,
    /// LLM text generation only.
    TextOnly,
    /// LLM text + image generation.
    Full,
}

impl Default for IntelligencePermission {
    fn default() -> Self {
        Self::None
    }
}
```

### AppPermissions Field

```rust
pub struct AppPermissions {
    // ... existing fields ...
    /// Intelligence (LLM / image gen) access level.
    #[serde(default)]
    pub intelligence: IntelligencePermission,
}
```

### Manifest Declaration

```toml
[app]
id = "parallax"
name = "Parallax"
version = "0.1.0"

[capabilities]
filesystem = "read_write"
intelligence = "full"       # none | text_only | full

[limits]
max_daily_usd = 5.00        # per-day spend cap for this app
max_session_usd = 2.00      # per-session spend cap (resets on app restart)
intelligence_tier = "medium" # default tier for requests that omit it
```

The `[limits]` section is required when `intelligence` is anything other than `none`. Missing limits with intelligence enabled is a manifest validation error — fail fast, don't invent defaults.

---

## 4. LLM Request/Response Protocol

All messages are newline-delimited JSON over the existing stdin/stdout channel.

### Request (App → Plexi)

```json
{
  "type": "llm_request",
  "request_id": "req_abc123",
  "tier": "medium",
  "system": "You are a video production assistant.",
  "messages": [
    {"role": "user", "content": "Write a 30s script for a product launch"},
    {"role": "assistant", "content": "Here's a draft..."},
    {"role": "user", "content": "Make it more energetic"}
  ],
  "max_tokens": 4096,
  "tools": []
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | Must be `"llm_request"` |
| `request_id` | string | yes | Caller-generated ID for correlating response |
| `tier` | string | no | `"low"` / `"medium"` / `"high"`. Falls back to manifest default |
| `system` | string | no | System prompt |
| `messages` | array | yes | Conversation history. Each entry: `{ role, content }` |
| `max_tokens` | integer | no | Max output tokens. Default: 4096 |
| `tools` | array | no | Tool definitions for tool-use flows (see Section 6) |

### Response (Plexi → App)

```json
{
  "type": "llm_response",
  "request_id": "req_abc123",
  "text": "Here's an energetic 30s script...",
  "input_tokens": 1200,
  "output_tokens": 450,
  "model": "claude-sonnet-4-6",
  "cost_usd": 0.012,
  "budget_remaining_session_usd": 1.988,
  "budget_remaining_daily_usd": 4.988
}
```

| Field | Type | Description |
|---|---|---|
| `type` | string | `"llm_response"` |
| `request_id` | string | Echoed from request |
| `text` | string or null | Generated text. Null when `stop_reason` is `"tool_use"` |
| `tool_calls` | array or null | Tool call requests (see Section 6) |
| `input_tokens` | integer | Tokens consumed by input |
| `output_tokens` | integer | Tokens generated |
| `model` | string | Actual model used |
| `cost_usd` | float | Cost of this request |
| `budget_remaining_session_usd` | float | Remaining session budget |
| `budget_remaining_daily_usd` | float | Remaining daily budget |
| `stop_reason` | string | `"end_turn"` / `"max_tokens"` / `"tool_use"` |

### Error Response

```json
{
  "type": "llm_error",
  "request_id": "req_abc123",
  "error": "session_budget_exceeded",
  "message": "Session budget of $2.00 exceeded. Current spend: $2.01",
  "budget_remaining_session_usd": 0.0,
  "budget_remaining_daily_usd": 2.87
}
```

Error codes:

| Code | Meaning |
|---|---|
| `session_budget_exceeded` | Per-session spend limit reached |
| `daily_budget_exceeded` | Per-day spend limit reached |
| `permission_denied` | App lacks `intelligence` capability |
| `tier_unavailable` | Requested tier has no configured model |
| `upstream_error` | API call to provider failed (includes provider error in `message`) |
| `invalid_request` | Malformed request (missing fields, bad types) |

---

## 5. Image Generation Request/Response Protocol

Requires `IntelligencePermission::Full`. Apps with `TextOnly` receive `permission_denied`.

### Request (App → Plexi)

```json
{
  "type": "image_gen_request",
  "request_id": "img_abc123",
  "prompt": "A professional product photo of a black foil pouch on marble surface",
  "output_path": "stills/scene_01.png",
  "dimensions": "1024x1024",
  "model_preference": "quality"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | Must be `"image_gen_request"` |
| `request_id` | string | yes | Caller-generated ID |
| `prompt` | string | yes | Image generation prompt |
| `output_path` | string | yes | Relative path (resolved against app scope root) |
| `dimensions` | string | no | `"WxH"` format. Default: `"1024x1024"` |
| `model_preference` | string | no | `"speed"` / `"quality"` / `"creative"`. Default: `"quality"` |

Model preference mapping (configured at Plexi level):

| Preference | Intent | Example Models |
|---|---|---|
| `speed` | Fast iteration, drafts | FLUX Schnell, Gemini Flash |
| `quality` | Production-ready output | Imagen 3, FLUX Pro |
| `creative` | Artistic, stylized | Gemini Imagen, Midjourney-style |

### Response (Plexi → App)

```json
{
  "type": "image_gen_response",
  "request_id": "img_abc123",
  "output_path": "stills/scene_01.png",
  "model": "gemini-2.0-flash",
  "cost_usd": 0.04,
  "budget_remaining_session_usd": 1.96,
  "budget_remaining_daily_usd": 4.96
}
```

The image is written to `output_path` (resolved relative to the app's scope root). The app reads it via `ReadFile` or directly from the filesystem if it has the appropriate permission.

### Error Response

Same format as LLM errors (`type: "image_gen_error"`), same error codes plus:

| Code | Meaning |
|---|---|
| `image_gen_not_permitted` | App has `TextOnly` intelligence, not `Full` |
| `output_path_denied` | Output path fails scope check or app lacks write permission |

---

## 6. Tool Use Protocol

For agentic apps that need multi-turn tool-calling loops. The app defines tools, the LLM decides when to call them, and the app executes them locally.

### Request with Tool Definitions

```json
{
  "type": "llm_request",
  "request_id": "req_tools_001",
  "tier": "medium",
  "system": "You are a video editor. Use the available tools to inspect and modify media files.",
  "messages": [
    {"role": "user", "content": "Trim clip.mp4 to the first 10 seconds"}
  ],
  "max_tokens": 4096,
  "tools": [
    {
      "name": "inspect_media",
      "description": "Get metadata about a media file (duration, resolution, codec)",
      "input_schema": {
        "type": "object",
        "properties": {
          "path": {"type": "string", "description": "Path to the media file"}
        },
        "required": ["path"]
      }
    },
    {
      "name": "trim_clip",
      "description": "Trim a video clip to a time range",
      "input_schema": {
        "type": "object",
        "properties": {
          "input_path": {"type": "string"},
          "output_path": {"type": "string"},
          "start_seconds": {"type": "number"},
          "end_seconds": {"type": "number"}
        },
        "required": ["input_path", "output_path", "start_seconds", "end_seconds"]
      }
    }
  ]
}
```

### Response with Tool Calls

When the model decides to use a tool, Plexi returns a response with `stop_reason: "tool_use"`:

```json
{
  "type": "llm_response",
  "request_id": "req_tools_001",
  "text": null,
  "tool_calls": [
    {
      "id": "tc_001",
      "name": "inspect_media",
      "input": {"path": "input/clip.mp4"}
    }
  ],
  "input_tokens": 800,
  "output_tokens": 120,
  "model": "claude-sonnet-4-6",
  "cost_usd": 0.005,
  "budget_remaining_session_usd": 1.995,
  "budget_remaining_daily_usd": 4.995,
  "stop_reason": "tool_use"
}
```

### Continuing with Tool Results

The app executes the tool locally, then sends the result back as a new `llm_request` with the tool result appended to the messages array:

```json
{
  "type": "llm_request",
  "request_id": "req_tools_002",
  "tier": "medium",
  "system": "You are a video editor...",
  "messages": [
    {"role": "user", "content": "Trim clip.mp4 to the first 10 seconds"},
    {"role": "assistant", "content": null, "tool_calls": [
      {"id": "tc_001", "name": "inspect_media", "input": {"path": "input/clip.mp4"}}
    ]},
    {"role": "tool", "tool_call_id": "tc_001", "content": "{\"duration\": 45.2, \"resolution\": \"1920x1080\", \"codec\": \"h264\"}"}
  ],
  "max_tokens": 4096,
  "tools": [...]
}
```

Plexi tracks cumulative cost across the full tool-use loop. Each turn is a separate `llm_request`/`llm_response` pair, and each turn's cost is deducted from the budget independently. If the budget runs out mid-loop, the app receives an `llm_error` and must handle the incomplete state.

---

## 7. Cost Tracking

### Cost Event Log

Every intelligence request is logged to `~/.plexi-alpha/costs.jsonl` (one JSON object per line):

```json
{
  "timestamp": "2026-04-11T14:32:01Z",
  "app_id": "parallax",
  "directory": "/Users/ian/projects/client-a",
  "request_type": "llm",
  "tier": "medium",
  "model": "claude-sonnet-4-6",
  "input_tokens": 1200,
  "output_tokens": 450,
  "cost_usd": 0.012,
  "session_total_usd": 0.45,
  "daily_total_usd": 2.13
}
```

For image generation requests, `request_type` is `"image_gen"` and token fields are omitted.

### Budget Enforcement

Budget checks happen **before** the API call. Plexi estimates the cost of the request (using input token count and max_tokens for worst-case output) and rejects if it would exceed the limit. After the actual response, the real cost is recorded.

| Limit | Scope | Reset |
|---|---|---|
| `max_session_usd` | Per app instance | When the app process restarts |
| `max_daily_usd` | Per app | Midnight local time |
| Global `max_daily_usd` | All apps combined | Midnight local time |

Priority (highest wins):

1. **Global daily limit** (`[intelligence].max_daily_usd` in Plexi config) — hard ceiling across all apps.
2. **Per-app daily limit** (`[limits].max_daily_usd` in manifest) — can only be lower than global.
3. **Per-app session limit** (`[limits].max_session_usd` in manifest) — resets each launch.

If the global limit is $20/day and an app declares $5/day, the app is capped at $5. If the global limit is $3/day, the app is capped at $3 regardless of its manifest.

### Session Tracking

Session state is held in memory per `ProcessApp` instance. When the app process exits, session spend resets. Daily spend is derived from `costs.jsonl` by summing entries for the current date and app_id.

---

## 8. Plexi System Configuration

```toml
# ~/.plexi-alpha/config.toml

[intelligence]
# Tier → model mapping (required if any app uses intelligence)
low_model = "claude-haiku-4-5"
medium_model = "claude-sonnet-4-6"
high_model = "claude-opus-4-6"

# Image generation model mapping
image_speed_model = "gemini-2.0-flash"
image_quality_model = "imagen-3"
image_creative_model = "gemini-2.0-flash"

# Global spend limit — hard ceiling across all apps
max_daily_usd = 20.00

# API keys — resolved from secrets manager (Keychain)
# These are secret names, not raw keys. Plexi fetches them at runtime.
anthropic_key_secret = "ANTHROPIC_API_KEY"
google_key_secret = "GOOGLE_API_KEY"
```

If the `[intelligence]` section is missing from config.toml, all intelligence requests return `tier_unavailable`. This is intentional — intelligence is opt-in at the platform level.

API keys are stored in the Keychain via the existing secrets system, namespaced to the Plexi system context (not per-app). The config references secret names, not values. Apps never see API keys.

---

## 9. Rust Implementation Notes

### New Types

```rust
// ─── app_permissions.rs ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligencePermission {
    None,
    TextOnly,
    Full,
}

impl Default for IntelligencePermission {
    fn default() -> Self {
        Self::None
    }
}

// Add to AppPermissions:
//   pub intelligence: IntelligencePermission,
```

```rust
// ─── app_api.rs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligenceTier {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageModelPreference {
    Speed,
    Quality,
    Creative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

// Add to ApiRequest enum:
pub enum ApiRequest {
    // ... existing variants ...
    LlmRequest {
        request_id: String,
        tier: Option<IntelligenceTier>,
        system: Option<String>,
        messages: Vec<Message>,
        max_tokens: Option<u32>,
        tools: Option<Vec<ToolDef>>,
    },
    ImageGenRequest {
        request_id: String,
        prompt: String,
        output_path: String,
        dimensions: Option<String>,
        model_preference: Option<ImageModelPreference>,
    },
}
```

```rust
// ─── config.rs ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct IntelligenceConfig {
    pub low_model: Option<String>,
    pub medium_model: Option<String>,
    pub high_model: Option<String>,
    pub image_speed_model: Option<String>,
    pub image_quality_model: Option<String>,
    pub image_creative_model: Option<String>,
    pub max_daily_usd: Option<f64>,
    pub anthropic_key_secret: Option<String>,
    pub google_key_secret: Option<String>,
}

// Add to PlexiConfig:
//   pub intelligence: Option<IntelligenceConfig>,
```

### Handler Flow

`handle_api_request` (or a new async variant) for intelligence requests:

1. **Permission check.** Read `intelligence` from `AppPermissions`. If `None`, return `permission_denied`. If `TextOnly` and request is `ImageGenRequest`, return `image_gen_not_permitted`.
2. **Config check.** Read `[intelligence]` from `PlexiConfig`. If missing, return `tier_unavailable`.
3. **Budget check.** Sum session spend (in-memory) and daily spend (from `costs.jsonl`). If either exceeds the limit, return `session_budget_exceeded` or `daily_budget_exceeded`.
4. **Resolve model.** Map tier → model name from config. Determine which API client to use (Anthropic vs Google) based on model name prefix.
5. **Fetch API key.** Read the secret name from config, fetch the actual key from Keychain via `secrets::get_secret`.
6. **Make API call.** Async HTTP request via `reqwest`. This must not block the event loop — run on a background task and send the response back via the app's stdout channel.
7. **Log cost event.** Append to `costs.jsonl`. Update in-memory session spend.
8. **Send response.** Serialize the response JSON and write to the app's stdin pipe.

### Async Consideration

Intelligence requests are inherently slow (seconds, not milliseconds). They must be handled asynchronously. The existing `handle_api_request` is synchronous. Two options:

- **Option A:** Add a separate async handler (`handle_intelligence_request`) that runs on a tokio task and sends the response via an mpsc channel back to the app's stdout writer.
- **Option B:** Make `handle_api_request` fully async (larger refactor, touches all existing handlers).

Option A is the right call for now — it's additive and doesn't touch working synchronous handlers.

---

## 10. Files to Modify

| File | Changes |
|---|---|
| `src/app_permissions.rs` | Add `IntelligencePermission` enum, add `intelligence` field to `AppPermissions`, update `builtin()` and `Default` impls |
| `src/app_api.rs` | Add `LlmRequest` and `ImageGenRequest` variants to `ApiRequest`, add intelligence response types, add async handler |
| `src/config.rs` | Add `IntelligenceConfig` struct, add `intelligence` field to `PlexiConfig`, add to config template |
| `src/process_app.rs` | Parse new JSON request types, route intelligence requests to async handler |
| `src/app_registry.rs` | Parse `[limits]` section from manifest, validate intelligence + limits co-requirement |
| `Cargo.toml` | Add `reqwest` (with `json` + `rustls-tls` features) if not already present |
| `docs/specs/subsystems/app-infrastructure.md` | Add intelligence to the capability list, add Phase 10 to the phase plan |

### New Files

| File | Purpose |
|---|---|
| `src/intelligence.rs` | API client abstraction (Anthropic + Google), cost calculation, budget enforcement, cost logging |
