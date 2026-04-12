# GitHub Issues — Plexi App

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Simple SDK (list, text, rect draw commands), text editor primitive (for comment authoring)  
**App type:** Out-of-process (Python), installable

---

## Summary

A keyboard-driven GitHub Issues browser that lives in a Plexi pane. Auto-detects the current git repo, fetches issues via `gh` CLI, and displays them in a clean, navigable interface. List view → detail view → comments, all without leaving the terminal.

No one's built this well as a TUI. The `gh` CLI outputs flat text. The web UI requires a browser switch. This app keeps you in Plexi.

---

## Why This App

- Proves Plexi apps can integrate with external tools (`gh` CLI) cleanly
- High daily-use value — issues are checked constantly during development
- Exercises the simple SDK's list + text rendering in a real workflow
- Natural integration point with pane labeling (issue title → pane label → Focus Manager priority)

---

## First-Run Setup

The app depends on the `gh` CLI being installed and authenticated. On every launch, it runs a three-step preflight check before showing any issues:

### Preflight Check

```
1. which gh          → is gh installed?
2. gh auth status    → is the user authenticated?
3. git remote get-url origin → are we in a GitHub repo?
```

Each failure gets its own dedicated screen — one problem, one action, one key to proceed.

### Screen: gh not installed

```
┌─────────────────────────────────────────────┐
│  GitHub Issues                              │
├─────────────────────────────────────────────┤
│                                             │
│  GitHub CLI (gh) is required but not found. │
│                                             │
│  Install it:                                │
│                                             │
│    brew install gh                          │
│                                             │
│  Press [r] to retry after installing.       │
│                                             │
└─────────────────────────────────────────────┘
```

### Screen: gh installed, not authenticated

```
┌─────────────────────────────────────────────┐
│  GitHub Issues                              │
├─────────────────────────────────────────────┤
│                                             │
│  GitHub CLI is installed but not logged in. │
│                                             │
│  Run this in your terminal:                 │
│                                             │
│    gh auth login                            │
│                                             │
│  [r] retry   [t] run in linked terminal     │
│                                             │
└─────────────────────────────────────────────┘
```

Pressing `[t]` emits `run_in_terminal("gh auth login")` so the user can authenticate in the linked terminal pane without leaving Plexi. After pressing `[t]`, the app polls `gh auth status` every 3 seconds and auto-transitions to the issue list once authentication succeeds.

### Screen: not in a GitHub repo

```
┌─────────────────────────────────────────────┐
│  GitHub Issues                              │
├─────────────────────────────────────────────┤
│                                             │
│  No GitHub repository detected.             │
│                                             │
│  Open this app from a directory with a      │
│  GitHub remote, or enter a repo manually:   │
│                                             │
│  Repo: [owner/repo                       ]  │
│                                             │
│  Recent:                                    │
│    ianjamesburke/PLEXI                      │
│    ianjamesburke/daily_log                  │
│                                             │
└─────────────────────────────────────────────┘
```

Recent repos are persisted to a local config file (`~/.plexi/apps/github-issues/state.json`) so you can quickly switch between repos you've used before.

---

## Data Source

Uses `gh` CLI (already installed, already authenticated). No direct GitHub API calls, no token management, no OAuth flow. The app shells out to `gh` and parses JSON output.

```bash
# List issues
gh issue list --repo owner/repo --json number,title,state,labels,author,assignees,milestone,updatedAt,createdAt --limit 50

# View single issue with body
gh issue view 42 --repo owner/repo --json number,title,state,body,labels,author,assignees,milestone,comments,createdAt,updatedAt

# List comments
gh issue view 42 --repo owner/repo --json comments
```

The repo is auto-detected from the pane's working directory (`git remote get-url origin`). If the pane was launched from a git repo, the app knows which repo to query. No configuration needed.

---

## Views

### List View (default)

```
┌─────────────────────────────────────────────────────────┐
│  ISSUES — ianjamesburke/PLEXI                    47 open│
├─────────────────────────────────────────────────────────┤
│  Filter: [all ▼]  [open ▼]  [search...          ]      │
├─────────────────────────────────────────────────────────┤
│  ● #89  Add bezier draw command              enhancement│
│    #87  App crash on resize with zoomed pane        bug │
│  ▸ #85  Text editor primitive                      idea │
│    #82  Capability system: phase 2 gate        P1   bug │
│    #79  Golden spiral layout preset                 idea│
│    #76  Focus manager app                          idea │
│    #71  SDK: mouse_move and mouse_up events   enhancement│
│    #68  Secrets: keychain integration              P2   │
│    ...                                                  │
├─────────────────────────────────────────────────────────┤
│  j/k navigate  Enter open  f filter  / search  n new   │
└─────────────────────────────────────────────────────────┘
```

**Layout:**

- **Header**: repo name (auto-detected), open issue count.
- **Filter bar**: dropdowns for label, state (open/closed/all), free-text search. Filters applied client-side after initial fetch (fast) with option to re-fetch with server-side filters for large repos.
- **Issue rows**: each row shows:
  - State indicator: `●` open (green), `○` closed (grey)
  - Issue number
  - Title (truncated to fit)
  - Primary label (rightmost, colored to match GitHub label color)
  - Priority label if present (P1/P2/P3/P4, using Plexi's priority colors)
  - Assignee avatar/initials (if space permits)
- **Selected row**: highlighted background (accent color), `▸` marker.
- **Footer**: keyboard shortcut hints.

**Styling:**

- Labels rendered with their actual GitHub color as background tint (the API returns hex colors for each label).
- Timestamps shown as relative ("2h ago", "3d ago") — not absolute dates.
- Unread/recently updated issues get a subtle bold treatment.

### Detail View

Opened by pressing Enter on a list item. Replaces the list view (not a new pane — same pane, different view. Backspace returns to list).

```
┌─────────────────────────────────────────────────────────┐
│  ← #82  Capability system: phase 2 gate                 │
│  bug  P1  opened by ianburke  3 days ago  2 comments    │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  The capability gate on `execute_app_command` needs to   │
│  check the app's declared permissions before allowing    │
│  filesystem or terminal operations.                      │
│                                                         │
│  ## Requirements                                        │
│  - Check `manifest.toml` capabilities against the       │
│    requested operation                                   │
│  - Block with a clear error if permission is missing     │
│  - Log all blocked attempts                              │
│                                                         │
│  ## Acceptance criteria                                  │
│  - [ ] Gate works for filesystem read/write              │
│  - [ ] Gate works for terminal_write                     │
│  - [ ] Blocked attempts logged at WARN level             │
│                                                         │
├─────────────────────────────────────────────────────────┤
│  COMMENTS (2)                                           │
├─────────────────────────────────────────────────────────┤
│  ianburke · 2 days ago                                  │
│  Started on this — the gate logic is in place but need   │
│  to handle the edge case where an app requests a         │
│  capability it didn't declare.                           │
│                                                         │
│  ianburke · 1 day ago                                   │
│  Fixed. PR #83 covers this. Leaving open until merged.   │
│                                                         │
├─────────────────────────────────────────────────────────┤
│  Backspace back  c comment  e edit  x close  o open web │
└─────────────────────────────────────────────────────────┘
```

**Layout:**

- **Title bar**: back arrow, issue number, title.
- **Metadata row**: labels (colored), priority, author, age, comment count.
- **Body**: full issue body with basic markdown rendering:
  - Headers rendered as bold text with size hierarchy
  - Code blocks rendered with syntax highlighting (monospace, dimmed background rect)
  - Checkboxes rendered as `[x]` / `[ ]`
  - Links shown as underlined text
  - Bold/italic preserved
  - No images (show `[image: alt text]` placeholder)
- **Comments section**: chronological, each with author + relative timestamp + body.
- **Footer**: keyboard shortcuts.

**Scrolling:** j/k scrolls the body and comments. The metadata row and title bar are pinned (always visible at top).

### New Issue / Comment Modal

Triggered by `n` (new issue from list view) or `c` (comment from detail view). Opens a modal overlay within the pane.

```
┌──────────────────────────────────────────┐
│  NEW ISSUE                               │
│                                          │
│  Title:                                  │
│  ┌──────────────────────────────────┐    │
│  │ Add text_editor draw command     │    │
│  └──────────────────────────────────┘    │
│                                          │
│  Labels: [enhancement ▼] [+ add]         │
│                                          │
│  Body:                                   │
│  ┌──────────────────────────────────┐    │
│  │ The text editor primitive needs  │    │
│  │ to be exposed as a draw command  │    │
│  │ so apps can embed editable text  │    │
│  │ fields.                          │    │
│  │                                  │    │
│  │ See spec: core-text-editor-      │    │
│  │ primitive.md                     │    │
│  └──────────────────────────────────┘    │
│                                          │
│       [Submit (Cmd+Enter)]  [Cancel]     │
└──────────────────────────────────────────┘
```

- **Title**: single-line text input.
- **Labels**: multi-select from repo's existing labels. Rendered as colored chips. Dropdown with search/filter.
- **Body**: text editor primitive in plain mode (markdown). Full editing, multi-line.
- **Submit**: `Cmd+Enter` creates the issue via `gh issue create`. Shows success confirmation with the new issue number, then navigates to the detail view.

For comments, same modal but without the title and labels fields — just the text editor and submit.

---

## Keyboard Navigation

### List View

| Key | Action |
|-----|--------|
| `j` / `↓` | Move selection down |
| `k` / `↑` | Move selection up |
| `Enter` | Open selected issue (detail view) |
| `/` | Focus search bar |
| `f` | Open filter dropdown |
| `l` | Filter by label (quick filter) |
| `a` | Filter by assignee |
| `n` | New issue modal |
| `r` | Refresh (re-fetch from GitHub) |
| `1` | Show open issues |
| `2` | Show closed issues |
| `3` | Show all issues |
| `o` | Open selected issue in browser (`gh issue view --web`) |
| `g` then `g` | Jump to top |
| `G` | Jump to bottom |
| `q` / `Escape` | Close app |

### Detail View

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll body/comments |
| `Backspace` | Back to list |
| `c` | New comment modal |
| `e` | Edit issue body (opens text editor with current body) |
| `x` | Close issue (with confirmation) |
| `u` | Reopen issue (if closed) |
| `o` | Open in browser |
| `l` | Edit labels |
| `p` | Set priority label (P1-P4, creates/updates label) |
| `y` | Copy issue URL to clipboard |

---

## Repo Detection

On launch, the app determines the GitHub repo:

1. Check if current directory is a git repo: `git rev-parse --is-inside-work-tree`
2. Get remote URL: `git remote get-url origin`
3. Parse `owner/repo` from the URL (handles both HTTPS and SSH formats)
4. Validate with `gh repo view owner/repo --json name` (confirms access)

If not in a git repo or no GitHub remote, show a prompt: "Enter a repo (owner/repo):" with a text input. Recently used repos are remembered for quick selection.

---

## Caching & Performance

- **Initial fetch**: loads first 50 open issues on app launch. Shown within ~1 second.
- **Background fetch**: after displaying the first page, fetches remaining issues in the background (pagination via `--limit` and cursor).
- **Cache**: issues are cached in memory for the session. `r` to refresh. Auto-refresh every 5 minutes if the app is visible.
- **Incremental updates**: on refresh, only fetch issues updated since last fetch (`--json updatedAt` comparison). Replace stale entries, append new ones.
- **Comment lazy-load**: comments are fetched when you open the detail view, not on list load. Keeps the initial load fast.

---

## Markdown Rendering

The issue body and comments are GitHub-flavored markdown. Full rendering in a terminal draw-command surface is complex. Pragmatic approach for MVP:

| Markdown element | Rendering |
|------------------|-----------|
| `# Heading` | Bold text, larger size |
| `## Subheading` | Bold text, same size |
| `**bold**` | Bold text |
| `*italic*` | Dimmed or italic (if font supports) |
| `` `inline code` `` | Monospace, slightly different background |
| ```` ```code block``` ```` | Monospace, dimmed background rect, syntax highlighting if language specified |
| `- list item` | Indented with bullet character |
| `- [ ] checkbox` | Rendered as `☐` / `☑` |
| `[link](url)` | Underlined text, accent color. Not clickable (show URL on hover or in footer) |
| `> quote` | Left border line + indented text |
| Images | `[image: alt_text]` placeholder |
| Tables | Monospace aligned columns (best-effort) |
| `---` | Horizontal line draw command |

This doesn't need to be pixel-perfect with GitHub's web rendering. It needs to be readable and navigable. The `o` shortcut opens the real GitHub page when you need full fidelity.

---

## Integration with Focus Manager

When viewing an issue in detail view, a keyboard shortcut (`p` for priority) lets you set a P1-P4 priority label directly on the issue. This:

1. Adds/updates a `P1`/`P2`/`P3`/`P4` label on the GitHub issue via `gh issue edit --add-label P1 --remove-label P2,P3,P4`.
2. If the Focus Manager is running, the pane label auto-updates to include the issue title and priority.

This creates a natural flow: browse issues → pick one → set priority → the Focus Manager tracks your attention on it.

---

## Integration with Pane Labels

When you open an issue in detail view, the pane label auto-updates to:

```
#82 Capability system: phase 2 gate
```

If the issue has a priority label (P1-P4), the pane priority is set to match. This means:
- The Focus Manager sees "you're working on a P1 issue" without any manual setup.
- The pane group outline (if linked) reflects the priority color.
- Cmd+Shift+L shows the issue title as the pane name.

---

## Manifest

```toml
[app]
id = "github-issues"
name = "GitHub Issues"
version = "0.1.0"
description = "Browse, create, and manage GitHub issues"

[capabilities]
filesystem = "read_only"    # read git config for repo detection
terminal_write = false
network = false             # uses gh CLI, not direct API calls

[app.handles]
commands = ["issues"]       # launched via command palette: "issues"
```

**Note on permissions:** This app doesn't need network access because it shells out to `gh` via `RunCommand`. The `gh` CLI handles authentication and network. The app only needs `RunCommand` capability and `filesystem.read_only` (to read `.git/config` for repo detection).

---

## File Structure

```
~/.plexi/apps/github-issues/
  manifest.toml
  github_issues.py
  plexi_sdk.py
```

Single file app. Target: under 500 lines (list view + detail view + comment modal + markdown renderer + caching).

---

## MVP Scope

1. **List view** — fetch open issues, render as navigable list with number/title/label/age, j/k navigation, Enter to open.
2. **Detail view** — issue body with basic markdown rendering, comments listed below, Backspace to return.
3. **Repo auto-detection** — parse owner/repo from git remote.
4. **Filter by state** — open/closed/all toggle via `1`/`2`/`3`.
5. **Open in browser** — `o` key opens the issue on github.com.

**Defer:** New issue modal, comment authoring, label editing, priority integration, search, assignee filter, milestone filter, incremental caching, auto-refresh, markdown table rendering.

---

## Future: PR View

The same app structure works for pull requests — `gh pr list`, `gh pr view`. Could be a tab within the same app (Issues | PRs toggle at the top) or a separate `github-prs` app. Same codebase, different `gh` subcommand. Defer until the issues app proves the pattern.
