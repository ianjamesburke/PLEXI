---
name: plexi-cli
description: Operating inside Plexi — spawn/name panes, focus, launch apps, surface notifications. Use when working in a Plexi pane or orchestrating other panes.
skill_version: "3.7.0"
plexi_version: "0.0.633"
---

# Plexi CLI

  You are running inside a Plexi pane. PLEXI_SOCKET is set automatically —
  every plexi command routes to the correct running instance.

  **Before using any subcommand you haven't used this session**, run `plexi
  <noun> --help` to confirm it exists and check its args signature. Subcommands
  change across releases — never assume.

  ## Panes

  `plexi pane new` is the primary spawner for terminal panes. `plexi terminal`
  is a legacy alias with the same flags but fewer options — prefer `pane new`.

    plexi pane self                        # print current pane ID (just the number)
    plexi pane list                        # JSON array: [{id, title, type, context_id, focused}]
    plexi pane list --context              # filter to caller's context (reads PLEXI_CONTEXT_ID)
    plexi pane list --context <id>         # filter to a specific context ID
    plexi pane info                        # JSON info for current pane (uses PLEXI_PANE_ID)
    plexi pane name "title"                # rename current pane
    plexi pane name <pane_id> "title"      # rename any pane by ID
    plexi pane focus <pane_id>             # move UI focus to a pane
    plexi pane close [pane_id]             # close pane (omit = current via PLEXI_PANE_ID)
    plexi pane send <pane_id> "text"       # inject text to PTY; \n works for shell, NOT Claude Code prompts
    plexi pane key <pane_id> <key>         # send keystroke: "enter", "escape", "ctrl+c"
    plexi pane capture [pane_id] --lines N # read last N lines of scrollback as JSON array (default 50)
    plexi pane capture [pane_id] --from-cursor <cursor>  # only lines written after cursor
    plexi pane command <pane_id> "text"    # send text to terminal pane
    plexi pane command <pane_id> "text" --enter  # send text + submit (appends newline)
    plexi pane state <pane_id>             # return current UI state as JSON (L1 UiNode tree for app panes)

  `plexi pane new` opens a terminal pane, returns pane ID on stdout.
  Direction flags (mutually exclusive): `-d` (below), `-l` (left), `-u` (up),
  `-r` (right, default), `--tab`, `--window`, `--overlay`.
  Other flags: `-n/--name <title>`, `--from <pane_id>`, `-e/--ephemeral`,
  `--no-focus`, `--cwd <path>`.

    LANE=$(plexi pane new -r)                        # pane to the right
    LANE=$(plexi pane new -r -n "dev server")        # named pane to the right
    TAB=$(plexi pane new --tab --from $LANE)         # new tab in LANE's window
    OV=$(plexi pane new --overlay)                   # overlay pane

  `plexi terminal [cmd]` — legacy alias. Flags: `-e/--ephemeral`, `--layout
  <LAYOUT>`, `--from-pane-id <id>`, `--cwd <path>`, `--no-focus`.
  Layout values: `split_h` (right), `split_left` (left), `split_right` (alias),
  `split_v` (below), `split_below` (alias), `split_above`, `tab`, `new_window`.

  `--from-pane-id` requires PLEXI_SOCKET. Use `$PLEXI_PANE_ID` (set
  automatically in every pane) — no extra round-trip needed.

  ## Apps

    plexi app list                         # list installed apps with versions
    plexi app open <type-id>               # open installed app by id
    plexi app open <path>                  # open local app directory (dev workflow)
    plexi app open --mcp <cmd> [args...]   # wrap stdio MCP server in a Plexi pane
    plexi app open --cli <binary>          # wrap CLI tool with visual UI
    plexi app open <type-id> [extra-args...] # pass extra args through to the app
    plexi app install <path>               # install a local app dir permanently
    plexi app install github:owner/repo    # install from GitHub
    plexi app install --pack core          # install from a pack file or built-in core pack
    plexi app install                      # install from workspace .plexi/apps.toml
    plexi app install <spec> --version 1.2.3  # pin to a specific version
    plexi app uninstall <app-id>           # remove installed app (-y to skip confirm)
    plexi app info <app-id>                # show id, name, version, available tools
    plexi app render <id>                  # render installed app to PNG (takes app ID, not path)
                                           # flags: --size WxH (default 800x600), --output <file.png>, --state <json>
    plexi app init <name>                  # scaffold a new app (--lang python, --global, --no-open, --from-pane-id)
    plexi app validate [path]              # validate app dir for errors (default: current dir)
    plexi app freeze <path>                # export installed apps as TOML snapshot (like pip freeze)
    plexi app publish                      # publish to marketplace (stub — future release)
    plexi app update [id]                  # check installed apps for available updates
    plexi app action <pane_id> <action> [args...]  # send semantic action to a running app pane

  `app open` direction flags (mutually exclusive): `-d` (below), `-l` (left),
  `-u` (up), `-r` (right), `--tab`, `--window`. Also `--from-pane-id <id>`.

  **Always use `plexi app open` for installed apps** — never `plexi pane send
  <id> "app\n"`. `app open` supports `--from-pane-id` and directional flags
  directly.

  `app render` takes an installed app ID, not a file path. Run `plexi app list`
  first to find the ID.

  2x2 grid example (apps in 3 quadrants):

    BALLS=$(plexi app open balls -r --from-pane-id $PLEXI_PANE_ID)
    plexi app open tetris -d --from-pane-id $PLEXI_PANE_ID
    plexi app open snake -d --from-pane-id $BALLS

  ## AI

    plexi ai doctor           # scan hardware, report recommended models (--json for scripting)
    plexi ai setup            # interactive Ollama setup wizard

  `ai doctor` detects CPU/RAM/VRAM/GPU, recommends local or cloud models,
  checks Ollama installation and pulled models, and verifies OpenRouter config.

  `ai setup` walks through Ollama installation, model recommendation based on
  your hardware, pulls the recommended model, and writes `[ai.ollama]` to
  `config.toml`.

  ## Agents

    plexi agent init <name>      # scaffold a new agent app with ai.query capability + chat UI
    plexi agent add <name>       # install agent definition from global registry into workspace
    plexi agent update <name>    # re-install agent definition, preserving memory and logs
    plexi agent list             # list agents installed in the current workspace

  `agent init` is equivalent to the former `plexi app init --agent <name>`.
  Agents live in `.plexi/agents/<name>/` with scoped memory and logs.

  ## Notifications

  Requires PLEXI_SOCKET. Flags: `--level info|warn|error`, `--timeout <seconds>`
  (0 = no timeout), `--scope <window|context|global>` (default: global).

  Choices via `--choice "key:Label"` (returns key). For host-side actions, add
  `--host-action "key:pane_focus:$PANE"`. Labels: 3-5 words, verb + object.

  **Blocking** (decision gate — cycle waits for user):

    RESULT=$(plexi notify --title "PR ready" --body "Summary." \
      --choice "a:Open PR" --choice "b:Skip" --choice "c:Talk to Claude" \
      --host-action "c:pane_focus:$MY_PANE")
    case "$RESULT" in
      a) open "<pr-url>" ;;
      b) ;; # skip
      c) ;; # pane_focus handled host-side
    esac

  **Fire-and-forget** (end-of-run only — background subshell, pane closes):

    (RESULT=$(plexi notify --title "Shipped" --body "PR merged" \
      --choice "ok:Dismiss" --choice "open:Open PR")
     [ "$RESULT" = "open" ] && open "<pr-url>") &

  ## Workspaces & Contexts

    plexi workspace init             # initialise a .plexi/ workspace in the current dir
                                     # never run from ~ — collides with profile dir

    plexi context new [name]         # create a new context (--path, --parent)
    plexi context open [path]        # switch current pane to a context at path
    plexi context set-root [path]    # set the root directory for the active context
    plexi context current            # print context ID and name as JSON
    plexi context describe <text>    # set the description for the active context
    plexi context zoom <context_id>  # zoom into a sub-context by numeric ID
    plexi context zoom-out           # zoom out to the parent context
    plexi context push [name]        # push focused pane into a new sub-context

  Every terminal pane has two env vars set at spawn time:

  - PLEXI_CONTEXT_ID — numeric ID of the context the pane belongs to
  - PLEXI_CONTEXT_NAME — display name of that context

  `plexi context current` reads these and prints `{"context_id": N,
  "context_name": "..."}`. Returns exit 1 if run outside a Plexi pane.

  ## Secrets

    plexi secret set <NAME>             # prompts for value; scoped to nearest .plexi/ workspace
    plexi secret set <NAME> --global    # store cross-workspace (global fallback)
    plexi secret set <NAME> --from-env  # read value from env var NAME instead of prompting
    plexi secret set <NAME> --alias <keychain-name>  # use different keychain entry name
    plexi secret get <NAME>             # print a stored secret's value to stdout
    plexi secret get <NAME> --global    # read from global store only
    plexi secret list                   # list stored secrets
    plexi secret delete <NAME>          # delete a workspace-scoped secret
    plexi secret delete <NAME> --global # delete a globally-stored secret

  ## Commands & Routines

    plexi run                        # list available commands from .plexi/commands.toml
    plexi run <command>              # run a named command from .plexi/commands.toml

    plexi routine list               # list routines from .plexi/routines.toml with schedule + next fire time
    plexi routine run <name>         # manually trigger a named routine

  ## Notes

    plexi notes list                 # print paths of all scratchpad notes, newest first
    plexi notes open                 # open note picker with fzf in the focused terminal pane

  Notes live at `<config_dir>/notes/` — one timestamped file per scratchpad
  session (Cmd+Shift+Space).

  ## Config

    plexi config check               # validate config.toml and report errors
    plexi config edit                # open config.toml in $EDITOR
    plexi config get <key>           # print resolved value of a config key (e.g. agents.medium)
    plexi config reset               # overwrite config.toml with built-in default template (backs up first)

  ## Registry

    plexi registry watch [cli]       # watch installed CLIs for descriptor drift

  ## System

    plexi doctor                     # audit all installed apps for capability and config gaps (--json)
    plexi demo                       # interactive keybinding tutorial — must be inside a Plexi pane
    plexi update                     # update Plexi itself
    plexi update apps [id]           # pull latest version of installed apps (omit id = all)
    plexi uninstall                  # uninstall Plexi binary, bundle, and completions (-y, --keep-data)
    plexi completions zsh            # print shell completion script (zsh, bash, fish)

  ## Cross-Pane Conversations

  Send messages to and read responses from coding assistants (Claude Code,
  Codex, Hermes, etc.) running in other panes. Assistant-agnostic — relies only
  on scrollback behavior, not UI-specific markers.

  ### Sending a message

  `pane send` injects text into the PTY. `\n` sends a raw newline byte —
  sufficient for **shell prompts**, but **not** for Claude Code's input handler.
  For Claude Code panes (idle at the Claude prompt), always follow with `pane key`:

    # Shell prompt — \n submits
    plexi pane send <pane_id> "git status\n"

    # Claude Code prompt — two steps required
    plexi pane send <pane_id> "your message here"
    plexi pane key <pane_id> enter

  Alternatively, use `plexi pane command <pane_id> "text" --enter` for terminal
  panes — equivalent to `pane send + \n` in one step.

  Avoid embedding `\n` mid-string for shell — the shell sees actual newlines and
  enters PS2 continuation mode.

  ### Waiting for a response

  The universal idle signal: **scrollback stopped changing.** Two identical
  captures with a gap = done.

    TARGET=<pane_id>
    PREV=""
    until [ -n "$PREV" ] && [ "$PREV" = "$(plexi pane capture $TARGET --lines 3)" ]; do
      PREV=$(plexi pane capture $TARGET --lines 3)
      sleep 3
    done

  ### Reading the response

  Capture the last N lines of scrollback. **Start with `--lines 80`.** If the
  response is clearly truncated (mid-sentence, no closing), step up to `--lines
  150` once. Never go higher — if 150 isn't enough, the response is too long to
  pull into your context anyway.

    plexi pane capture <pane_id> --lines 80

  To extract only content after the message you sent:

    plexi pane capture <pane_id> --lines 80 | \
      sed -n '/your message/,$ p' | tail -n +2

  ### Full send-wait-read pattern

    TARGET=<pane_id>
    MSG="your question here"

    # 1. Send (Claude Code pane — two steps)
    plexi pane send $TARGET "$MSG"
    plexi pane key $TARGET enter

    # 2. Wait for idle (scrollback stabilizes)
    PREV=""
    until [ -n "$PREV" ] && [ "$PREV" = "$(plexi pane capture $TARGET --lines 3)" ]; do
      PREV=$(plexi pane capture $TARGET --lines 3)
      sleep 3
    done

    # 3. Read response
    plexi pane capture $TARGET --lines 80
