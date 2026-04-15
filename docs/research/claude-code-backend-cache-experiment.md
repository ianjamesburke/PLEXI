# Claude Code `-p --resume` Cache Experiment

## Question
Does `claude -p --resume <session-id>` preserve Anthropic prompt caching across invocations, enough to justify rebuilding Plexi's agent mode as a Claude Code subprocess wrapper?

## Experiment
All commands run from a fresh shell, `--output-format json`.

```sh
# Step 1: baseline
claude -p --output-format json "What is 2+2? Answer in one word."

# Step 2 & 3: resume same session
claude -p --output-format json --resume <sid> "Now what is 3+3?"
claude -p --output-format json --resume <sid> "And 5+5?"

# Step 4: larger context with file read, then resume
claude -p --output-format json "Read this file: .../docs/specs/agent-mode.md. Summarize it in one sentence."
claude -p --output-format json --resume <sid> "What section of that file covers trust scoring?"

# Step 5: tool use
claude -p --output-format json "Run 'ls /tmp' and tell me how many files there are."
claude -p "..." --output-format json --permission-mode bypassPermissions
```

## Results

**Step 1 — baseline (sid `7d026b6a...`)**
```json
"usage": { "input_tokens": 3, "cache_creation_input_tokens": 24161, "cache_read_input_tokens": 0, "output_tokens": 4 }
"total_cost_usd": 0.09067
```

**Step 2 — first resume**
```json
"usage": { "input_tokens": 3, "cache_creation_input_tokens": 15, "cache_read_input_tokens": 24161, "output_tokens": 4 }
"total_cost_usd": 0.00737
```

**Step 3 — second resume**
```json
"usage": { "input_tokens": 3, "cache_creation_input_tokens": 13, "cache_read_input_tokens": 24176, "output_tokens": 4 }
"total_cost_usd": 0.00737
```

**Step 4a — file read baseline (sid `56a39bae...`)**
```json
"usage": { "input_tokens": 4, "cache_creation_input_tokens": 12426, "cache_read_input_tokens": 36706, "output_tokens": 182, "num_turns": 2 }
```

**Step 4b — resume with file follow-up**
```json
"usage": { "input_tokens": 4, "cache_creation_input_tokens": 362, "cache_read_input_tokens": 49967, "output_tokens": 178 }
"total_cost_usd": 0.01903
```

**Step 5a — default perms (blocked)**
```json
"permission_denials": [{ "tool_name": "Bash", "tool_input": { "command": "ls /tmp | wc -l" } }]
"result": "The command was blocked..."
```

**Step 5b — `--permission-mode bypassPermissions`**
```json
"usage": { "cache_read_input_tokens": 48330, "cache_creation_input_tokens": 97 }
"result": "There are 649 items in `/tmp`."
```
Ground-truth `ls /tmp | wc -l` = 649. Exact match — no hallucination.

## Verdict
**cache_survives: yes.** The cost drop from $0.09067 → $0.00737 (92% cheaper) on the second turn is unambiguous. `cache_read_input_tokens` grew from 24161 → 24176 → 49967 across turns, proving Claude Code sets `cache_control: ephemeral` on the running conversation history and the prior assistant turn gets folded into the cached prefix on each resume. This holds for both tiny chats and larger contexts that include tool results from file reads. Cost per incremental turn is essentially cache-read pricing.

## Tool use
**tools_work_in_p_mode: yes.** `-p` mode ran Read (step 4) and Bash (step 5b) tools successfully — `num_turns: 2` and exact file contents / `ls /tmp` count confirm real execution, not hallucination. Default permission mode blocks Bash unless you pass `--permission-mode bypassPermissions` (or a matching `--allowedTools` pattern, which is finicky — `Bash(ls:*)` did not match `ls /tmp | wc -l`). Plexi would need to choose a permission policy and pass it on every invocation.

## Recommendation
**Yes — rebuild agent mode as a `claude -p --resume` wrapper.** This eliminates the custom Anthropic client (ureq/JSON/auth), gets prompt caching, tool use (Read/Edit/Bash/Grep), and user auth for free, and slashes recurring-turn cost by ~92%. Migration: spawn `claude -p --output-format stream-json --resume <sid> --permission-mode <policy>` per user turn, pipe stdout into the existing output panel, persist `session_id` in Plexi's pane state, show `permission_denials` as in-pane prompts the user can re-run with an allowed command. Keep `agent_llm.rs` around for one release as a fallback if the Claude CLI is missing, then delete.

## Gotchas
- Every `-p` invocation is a fresh process; the session is re-loaded from disk. `session_id` is stable across `--resume` calls and returned in every JSON response — persist it per pane.
- First turn always shows a ~24k-token `cache_creation` — that's the Claude Code system prompt / tool definitions. Unavoidable baseline cost (~$0.09) on every new session. Budget for it.
- Default permission mode silently blocks Bash; returns a `permission_denials` array and a conversational "blocked" result. Plexi must pick `--permission-mode bypassPermissions` (Plexi's own trust system gates it) or craft precise `--allowedTools` patterns (note: `Bash(ls:*)` did **not** match `ls /tmp | wc -l` — pipes break the pattern).
- `--output-format json` emits a single blob at end of turn; use `stream-json` for live streaming into Plexi's output pane.
- Cache TTL is 5 minutes (`ephemeral_5m`). Long idle gaps between turns will re-pay the cache-creation cost.
- `--allowed-tools` (hyphen) errors out as unknown; the working flag is `--allowedTools` (camelCase) and it must come **after** the prompt argument.
- No rate-limit issues observed; auth is shared with interactive mode (same user session).
