from __future__ import annotations
"""
github-issues — Plexi app

Keyboard-driven GitHub Issues browser. Shells out to the `gh` CLI for all
data, so the app needs `gh` installed, authenticated, and a GitHub remote
in its launch directory.

Phase 1 SDK components layer: uses ctx.header, ctx.status_bar,
ctx.scrollable_list, ctx.scrollable_text, ctx.empty_state, ctx.wrap_text,
THEME, and named size constants.

Controls (List view):
  j / ArrowDown   Next issue
  k / ArrowUp     Previous issue
  Enter           Open detail view for selected issue
  o               Filter: open issues
  c               Filter: closed issues
  r               Refresh from GitHub

Controls (Detail view):
  j / ArrowDown   Scroll body / comments down
  k / ArrowUp     Scroll body / comments up
  Backspace       Back to list

Controls (Error / preflight failure):
  r               Re-run preflight checks
"""

import json
import os
import queue
import shutil
import subprocess
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (  # noqa: E402
    App,
    THEME,
    BODY, CAPTION, HINT, MONO_BODY,
    PAD, HEADER_H,
)

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

VIEW_LOADING = "loading"
VIEW_ERROR   = "error"
VIEW_LIST    = "list"
VIEW_DETAIL  = "detail"

view: str = VIEW_LOADING
loading_msg: str = "Loading\u2026"

# Preflight error state. {"title", "message", "fix_command"}
error: dict | None = None

# Repo info, populated after preflight succeeds.
repo_owner: str | None = None
repo_name: str | None = None

# List view state
issue_state_filter: str = "open"   # "open" | "closed"
issues: list[dict] = []
selected: int = 0

# Detail view state
detail_issue_number: int | None = None
detail_data: dict | None = None    # {body, comments}
detail_scroll: int = 0

# Cross-thread message queue (worker -> render).
result_queue: "queue.Queue[dict]" = queue.Queue()

# Priority label accent colors (app-specific; not in the shared Theme).
PRIORITY_COLORS = {
    "P1": THEME.red,
    "P2": THEME.yellow,
    "P3": THEME.accent,
    "P4": THEME.muted,
}

# ---------------------------------------------------------------------------
# gh CLI helpers — all run on a worker thread
# ---------------------------------------------------------------------------

def _gh_bin() -> str:
    """Resolve the gh binary. Honors PLEXI_GH_BIN for tests."""
    return os.environ.get("PLEXI_GH_BIN") or "gh"


def _run(cmd: list[str], timeout: float = 10.0) -> tuple[int, str, str]:
    """Run a subprocess. Returns (returncode, stdout, stderr). Never raises."""
    try:
        r = subprocess.run(
            cmd, capture_output=True, text=True, check=False, timeout=timeout,
        )
        return r.returncode, r.stdout, r.stderr
    except FileNotFoundError as e:
        return 127, "", f"command not found: {cmd[0]} ({e})"
    except subprocess.TimeoutExpired as e:
        return 124, "", f"timed out after {timeout}s: {' '.join(cmd)} ({e})"
    except Exception as e:
        return 1, "", f"failed to run {' '.join(cmd)}: {e}"


def _parse_owner_repo(url: str) -> tuple[str, str] | None:
    """Parse owner/repo from a GitHub remote URL (HTTPS or SSH)."""
    url = url.strip()
    if url.endswith(".git"):
        url = url[:-4]
    if url.startswith("git@github.com:"):
        path = url.split(":", 1)[1]
    elif "github.com/" in url:
        path = url.split("github.com/", 1)[1]
    else:
        return None
    parts = path.split("/")
    if len(parts) >= 2 and parts[0] and parts[1]:
        return parts[0], parts[1]
    return None


def run_preflight():
    """Three-step preflight: gh installed, gh authed, cwd is GitHub repo."""
    gh = _gh_bin()
    if not (os.path.isabs(gh) and os.path.exists(gh)) and shutil.which(gh) is None:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "GitHub CLI not found",
            "message": "GitHub CLI (gh) is required but not installed.",
            "fix_command": "brew install gh",
        }})
        return

    rc, _, err = _run([gh, "auth", "status"], timeout=5.0)
    if rc != 0:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "GitHub CLI not authenticated",
            "message": "GitHub CLI is installed but not logged in.",
            "fix_command": "gh auth login",
        }})
        return

    rc, out, err = _run(["git", "remote", "get-url", "origin"], timeout=5.0)
    if rc != 0:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "Not a GitHub repo",
            "message": "No git remote 'origin' found in this directory. "
                       "Launch the app from inside a GitHub repository.",
            "fix_command": "cd <your-repo>",
        }})
        return

    parsed = _parse_owner_repo(out)
    if not parsed:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "Not a GitHub repo",
            "message": f"Remote origin is not a GitHub URL:\n  {out.strip()}",
            "fix_command": "git remote set-url origin <github-url>",
        }})
        return

    owner, name = parsed
    result_queue.put({"action": "preflight_ok", "owner": owner, "name": name})


def fetch_issues(state: str):
    """Fetch issues from gh and push to queue."""
    gh = _gh_bin()
    rc, out, err = _run(
        [gh, "issue", "list",
         "--state", state,
         "--json", "number,title,state,labels,author",
         "--limit", "50"],
        timeout=15.0,
    )
    if rc != 0:
        result_queue.put({"action": "error", "message": f"gh issue list failed: {err.strip() or out.strip()}"})
        return
    try:
        data = json.loads(out)
    except json.JSONDecodeError as e:
        result_queue.put({"action": "error", "message": f"failed to parse gh JSON: {e}"})
        return
    if not isinstance(data, list):
        result_queue.put({"action": "error", "message": f"unexpected gh response shape: {type(data).__name__}"})
        return
    result_queue.put({"action": "issues_done", "items": data})


def fetch_issue_detail(number: int):
    """Fetch single issue body+comments."""
    gh = _gh_bin()
    rc, out, err = _run(
        [gh, "issue", "view", str(number), "--json", "body,comments"],
        timeout=15.0,
    )
    if rc != 0:
        rc2, out2, err2 = _run(
            [gh, "issue", "view", str(number), "--json", "body"],
            timeout=15.0,
        )
        if rc2 != 0:
            result_queue.put({"action": "error",
                              "message": f"gh issue view failed: {err.strip() or err2.strip()}"})
            return
        out = out2
    try:
        data = json.loads(out)
    except json.JSONDecodeError as e:
        result_queue.put({"action": "error", "message": f"failed to parse gh JSON: {e}"})
        return
    result_queue.put({
        "action": "detail_done",
        "number": number,
        "body": data.get("body", "") or "",
        "comments": data.get("comments", []) or [],
    })


def start_worker(fn, *args):
    threading.Thread(target=fn, args=args, daemon=True).start()


# ---------------------------------------------------------------------------
# State transitions
# ---------------------------------------------------------------------------

def begin_preflight():
    global view, loading_msg
    view = VIEW_LOADING
    loading_msg = "Checking gh CLI\u2026"
    start_worker(run_preflight)


def begin_fetch_issues():
    global view, loading_msg
    view = VIEW_LOADING
    loading_msg = "Loading issues\u2026"
    start_worker(fetch_issues, issue_state_filter)


def begin_fetch_detail(number: int):
    global view, loading_msg, detail_issue_number, detail_data, detail_scroll
    view = VIEW_LOADING
    loading_msg = f"Loading issue #{number}\u2026"
    detail_issue_number = number
    detail_data = None
    detail_scroll = 0
    start_worker(fetch_issue_detail, number)


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------

app = App()


def _drain_queue():
    global view, error, repo_owner, repo_name, issues, selected, detail_data
    try:
        while True:
            msg = result_queue.get_nowait()
            action = msg["action"]
            if action == "preflight_ok":
                repo_owner = msg["owner"]
                repo_name = msg["name"]
                error = None
                begin_fetch_issues()
            elif action == "preflight_fail":
                error = msg["error"]
                view = VIEW_ERROR
            elif action == "issues_done":
                issues = msg["items"]
                selected = 0
                view = VIEW_LIST
            elif action == "detail_done":
                detail_data = {"body": msg["body"], "comments": msg["comments"]}
                view = VIEW_DETAIL
            elif action == "error":
                error = {
                    "title": "Error",
                    "message": msg["message"],
                    "fix_command": "",
                }
                view = VIEW_ERROR
    except queue.Empty:
        pass


@app.on_render
def render(ctx):
    _drain_queue()

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    if view == VIEW_LOADING:
        _render_loading(ctx)
    elif view == VIEW_ERROR:
        _render_error(ctx)
    elif view == VIEW_DETAIL:
        _render_detail(ctx)
    else:
        _render_list(ctx)


# ── loading ─────────────────────────────────────────────────────────────────
def _render_loading(ctx):
    ctx.header("GitHub Issues")
    ctx.empty_state(loading_msg, icon_color=THEME.accent)
    ctx.status_bar([("⌘W", "close")])


# ── error ───────────────────────────────────────────────────────────────────
def _render_error(ctx):
    ctx.header("GitHub Issues")
    if not error:
        ctx.status_bar([("r", "retry"), ("⌘W", "close")])
        return

    y = HEADER_H + PAD
    ctx.text(PAD, y, error.get("title", "Error"),
             size=BODY, color=THEME.red, bold=True)
    y += BODY + 12

    msg = error.get("message", "")
    for line in ctx.wrap_text(msg, max_width_px=ctx.width - PAD * 2, size=CAPTION):
        ctx.text(PAD, y, line, size=CAPTION, color=THEME.fg)
        y += CAPTION + 4

    fix = error.get("fix_command", "")
    if fix:
        y += 12
        ctx.text(PAD, y, "Fix:", size=HINT, color=THEME.muted)
        y += HINT + 6
        fix_w = ctx.measure_text(fix, size=MONO_BODY, monospace=True)
        box_w = min(fix_w + 24, ctx.width - PAD * 2)
        ctx.rect(PAD, y - 4, box_w, MONO_BODY + 12, fill=THEME.surface, radius=4.0)
        ctx.text(PAD + 12, y, fix, size=MONO_BODY, color=THEME.green, monospace=True)

    ctx.status_bar([("r", "retry preflight"), ("⌘W", "close")])


# ── list ────────────────────────────────────────────────────────────────────
def _list_shortcuts() -> list[tuple[str, str]]:
    return [
        ("j/k", "navigate"),
        ("Enter", "open"),
        ("o", "open"),
        ("c", "closed"),
        ("r", "refresh"),
        ("⌘W", "close"),
    ]


def _label_chip(label: dict) -> tuple[str, str]:
    """Return (display_name, hex_color) for a label."""
    name = label.get("name", "")
    if name in PRIORITY_COLORS:
        return name, PRIORITY_COLORS[name]
    color_hex = label.get("color") or ""
    if color_hex and not color_hex.startswith("#"):
        color_hex = "#" + color_hex
    if not color_hex:
        color_hex = THEME.surface
    return name, color_hex


def _render_issue_row(ctx, issue, _idx, x, y, w, is_sel):
    row_h = 52.0
    bg = THEME.highlight if is_sel else THEME.bg
    ctx.rect(x + PAD / 2, y, w - PAD, row_h, fill=bg, radius=6)

    number = issue.get("number", 0)
    title_text = issue.get("title", "(no title)")
    state = issue.get("state", "OPEN")
    author = (issue.get("author") or {}).get("login", "")
    labels = issue.get("labels") or []

    # State dot.
    dot_color = THEME.green if state == "OPEN" else THEME.muted
    ctx.rect(x + PAD, y + (row_h - 8) / 2, 8, 8, fill=dot_color, radius=4)

    # #number
    num_text = f"#{number}"
    num_x = x + PAD + 20
    ctx.text(num_x, y + 8, num_text, size=CAPTION,
             color=THEME.muted, monospace=True)
    num_w = ctx.measure_text(num_text, size=CAPTION, monospace=True)

    # Right-side author + labels reservation.
    right_edge = x + w - PAD
    author_w = ctx.measure_text(author, size=HINT) if author else 0.0
    author_x = right_edge - author_w
    if author:
        ctx.text(author_x, y + 8, author, size=HINT, color=THEME.muted)

    # Labels — right-to-left, starting left of the author name.
    label_cursor = author_x - (12 if author else 0)
    for lbl in reversed(labels[:4]):
        chip_text, chip_color = _label_chip(lbl)
        chip_w = ctx.measure_text(chip_text, size=HINT) + 12
        label_cursor -= chip_w + 6
        ctx.rect(label_cursor, y + 6, chip_w, HINT + 8, fill=chip_color, radius=4.0)
        ctx.text(label_cursor + 6, y + 8, chip_text, size=HINT, color=THEME.bg)

    # Title — truncate to fit remaining space.
    title_x = num_x + num_w + 12
    max_title_w = max(40, label_cursor - title_x - 8)
    wrapped = ctx.wrap_text(title_text, max_width_px=max_title_w, size=BODY)
    line = wrapped[0] if wrapped else title_text
    if len(wrapped) > 1:
        line = line.rstrip() + "\u2026"
    title_color = THEME.fg if is_sel else THEME.fg
    ctx.text(title_x, y + 8, line, size=BODY,
             color=title_color, bold=is_sel)

    # Second row: subtitle (state + author summary or labels list).
    sub_parts = [state.lower()]
    if labels:
        sub_parts.append(", ".join(l.get("name", "") for l in labels[:3]))
    sub = "  ·  ".join(p for p in sub_parts if p)
    ctx.text(title_x, y + 8 + BODY + 4, sub[:200],
             size=CAPTION, color=THEME.muted)


def _render_list(ctx):
    repo_str = f"{repo_owner}/{repo_name}" if repo_owner else "GitHub Issues"
    count = len(issues)
    title = f"Issues  ·  {repo_str}"
    subtitle = f"{count} {issue_state_filter}"
    ctx.header(title, subtitle=subtitle)

    if not issues:
        ctx.empty_state(
            f"No {issue_state_filter} issues",
            subtitle="Press r to refresh",
        )
        ctx.status_bar(_list_shortcuts())
        return

    ctx.scrollable_list(
        list_id="issues",
        items=issues,
        selected=selected,
        row_height=56.0,
        render_row=_render_issue_row,
    )

    ctx.status_bar(_list_shortcuts())


# ── detail ──────────────────────────────────────────────────────────────────
def _render_detail(ctx):
    global detail_scroll

    cached = next((i for i in issues if i.get("number") == detail_issue_number), None)
    title_text = cached.get("title", f"#{detail_issue_number}") if cached else f"#{detail_issue_number}"
    header_title = f"#{detail_issue_number}  {title_text}"
    ctx.header(header_title)

    if not detail_data:
        ctx.empty_state("Loading\u2026", icon_color=THEME.accent)
        ctx.status_bar([
            ("j/k", "scroll"),
            ("Backspace", "back"),
            ("⌘W", "close"),
        ])
        return

    max_w = ctx.width - PAD * 2
    body = detail_data.get("body") or ""
    lines: list[str] = list(ctx.wrap_text(body, max_width_px=max_w,
                                          size=MONO_BODY, monospace=True))

    comments = detail_data.get("comments") or []
    if comments:
        lines.append("")
        lines.append(f"COMMENTS ({len(comments)})")
        lines.append("\u2500" * 60)
        for c in comments:
            author = (c.get("author") or {}).get("login", "")
            created = (c.get("createdAt") or "")[:10]
            head = f"{author} \u00b7 {created}".strip(" \u00b7")
            lines.append(head)
            for line in ctx.wrap_text(c.get("body") or "", max_width_px=max_w,
                                       size=MONO_BODY, monospace=True):
                lines.append(line)
            lines.append("")

    detail_scroll = ctx.scrollable_text(
        text_id="issue_detail",
        lines=lines,
        scroll_offset=detail_scroll,
        line_height=MONO_BODY + 4,
        size=MONO_BODY,
        monospace=True,
    )

    ctx.status_bar([
        ("j/k", "scroll"),
        ("Backspace", "back"),
        ("⌘W", "close"),
    ])


# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------

@app.on_key
def on_key(key, _mods, _emit):
    global view, selected, detail_scroll, issue_state_filter, error

    if view == VIEW_LOADING:
        return

    if view == VIEW_ERROR:
        if key == "r":
            begin_preflight()
        return

    if view == VIEW_DETAIL:
        if key == "Backspace":
            view = VIEW_LIST
        elif key in ("j", "ArrowDown"):
            detail_scroll += 1
        elif key in ("k", "ArrowUp"):
            detail_scroll = max(0, detail_scroll - 1)
        return

    # VIEW_LIST
    if key in ("j", "ArrowDown"):
        if issues:
            selected = min(selected + 1, len(issues) - 1)
    elif key in ("k", "ArrowUp"):
        if issues:
            selected = max(selected - 1, 0)
    elif key == "Enter":
        if issues and 0 <= selected < len(issues):
            begin_fetch_detail(int(issues[selected]["number"]))
    elif key == "r":
        begin_fetch_issues()
    elif key == "o":
        if issue_state_filter != "open":
            issue_state_filter = "open"
            begin_fetch_issues()
    elif key == "c":
        if issue_state_filter != "closed":
            issue_state_filter = "closed"
            begin_fetch_issues()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

begin_preflight()
app.run()
