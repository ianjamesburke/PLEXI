#!/usr/bin/env python3
"""
github-issues — Plexi app

Keyboard-driven GitHub Issues browser. Shells out to the `gh` CLI for all
data, so the app needs `gh` installed, authenticated, and a GitHub remote
in its launch directory.

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

MVP scope. Deferred (need text editor primitive or follow-up issues):
  - Comment authoring, body editing, label/state mutation
  - Search, assignee/milestone filters
  - Pane label integration, priority Focus Manager hand-off
"""

import json
import os
import queue
import shutil
import subprocess
import sys
import textwrap
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

VIEW_LOADING  = "loading"
VIEW_ERROR    = "error"
VIEW_LIST     = "list"
VIEW_DETAIL   = "detail"

view: str = VIEW_LOADING
loading_msg: str = "Loading\u2026"

# Preflight error state. {"title", "message", "fix_command"}
error: dict | None = None

# Repo info, populated after preflight succeeds.
repo_owner: str | None = None
repo_name: str | None = None

# List view state
issue_state_filter: str = "open"   # "open" | "closed"
issues: list[dict] = []            # list of {number,title,state,labels,author}
selected: int = 0
list_scroll: int = 0

# Detail view state
detail_issue_number: int | None = None
detail_data: dict | None = None    # {body, comments}
detail_scroll: int = 0

# Cross-thread message queue (worker -> render).
result_queue: "queue.Queue[dict]" = queue.Queue()

# ---------------------------------------------------------------------------
# Theme — Catppuccin Mocha (matches wikipedia/git-log apps)
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "header":   "#181825",
    "surface":  "#313244",
    "sel_bg":   "#45475a",
    "text":     "#cdd6f4",
    "subtext":  "#a6adc8",
    "muted":    "#6c7086",
    "accent":   "#89b4fa",
    "green":    "#a6e3a1",
    "red":      "#f38ba8",
    "orange":   "#fab387",
    "blue":     "#89b4fa",
    "grey":     "#7f849c",
    "yellow":   "#f9e2af",
}

PRIORITY_COLORS = {
    "P1": C["red"],
    "P2": C["orange"],
    "P3": C["blue"],
    "P4": C["grey"],
}

HEADER_H = 52
FOOTER_H = 28
PADDING  = 16
ITEM_H   = 44
LINE_H   = 18

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
    # git@github.com:owner/repo
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
    # 1. gh installed?
    if not (os.path.isabs(gh) and os.path.exists(gh)) and shutil.which(gh) is None:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "GitHub CLI not found",
            "message": "GitHub CLI (gh) is required but not installed.",
            "fix_command": "brew install gh",
        }})
        return

    # 2. gh authenticated?
    rc, _, err = _run([gh, "auth", "status"], timeout=5.0)
    if rc != 0:
        result_queue.put({"action": "preflight_fail", "error": {
            "title": "GitHub CLI not authenticated",
            "message": "GitHub CLI is installed but not logged in.",
            "fix_command": "gh auth login",
        }})
        return

    # 3. inside a GitHub repo?
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
    # Try with comments first; fall back to body-only if comments unsupported.
    rc, out, err = _run(
        [gh, "issue", "view", str(number), "--json", "body,comments"],
        timeout=15.0,
    )
    if rc != 0:
        # Fall back: body-only.
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


@app.on_render
def render(ctx):
    global view, error, repo_owner, repo_name, issues, selected, detail_data, list_scroll

    # Drain worker messages.
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
                list_scroll = 0
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

    # Background.
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    if view == VIEW_LOADING:
        _render_loading(ctx)
    elif view == VIEW_ERROR:
        _render_error(ctx)
    elif view == VIEW_DETAIL:
        _render_detail(ctx)
    else:
        _render_list(ctx)


def _draw_header(ctx, title: str, hint: str):
    w = ctx.width
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, title, size=14, color=C["accent"], bold=True)
    if hint:
        hint_x = w - len(hint) * 7.0 - PADDING
        if hint_x > PADDING + len(title) * 8:
            ctx.text(hint_x, 16, hint, size=11, color=C["muted"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)


def _draw_footer(ctx, hint: str):
    w = ctx.width
    h = ctx.height
    ctx.rect(0, h - FOOTER_H, w, FOOTER_H, fill=C["header"])
    ctx.line(0, h - FOOTER_H, w, h - FOOTER_H, color=C["surface"], width=1.0)
    ctx.text(PADDING, h - FOOTER_H + 8, hint, size=11, color=C["muted"])


def _render_loading(ctx):
    _draw_header(ctx, "GitHub Issues", "")
    ctx.text(PADDING, HEADER_H + 20, loading_msg, size=13, color=C["subtext"])


def _render_error(ctx):
    _draw_header(ctx, "GitHub Issues", "")
    if not error:
        return
    y = HEADER_H + 24
    ctx.text(PADDING, y, error.get("title", "Error"),
             size=15, color=C["red"], bold=True)
    y += 28

    # Word-wrap the message.
    msg = error.get("message", "")
    char_width = max(20, int((ctx.width - 2 * PADDING) / 7.5))
    for line in wrap_text(msg, char_width):
        ctx.text(PADDING, y, line, size=13, color=C["text"])
        y += LINE_H

    fix = error.get("fix_command", "")
    if fix:
        y += 12
        ctx.text(PADDING, y, "Fix:", size=12, color=C["muted"])
        y += LINE_H
        # Fix command in a subtle box.
        box_w = min(len(fix) * 9 + 24, ctx.width - 2 * PADDING)
        ctx.rect(PADDING, y - 4, box_w, LINE_H + 8, fill=C["surface"], radius=4.0)
        ctx.text(PADDING + 12, y, fix, size=13, color=C["green"], monospace=True)

    _draw_footer(ctx, "[r] retry preflight")


def _render_list(ctx):
    global list_scroll
    w = ctx.width
    h = ctx.height

    # Header
    repo_str = f"{repo_owner}/{repo_name}" if repo_owner else "GitHub Issues"
    count = len(issues)
    title = f"ISSUES — {repo_str}    {count} {issue_state_filter}"
    hint = "j/k navigate  Enter open  o open  c closed  r refresh"
    _draw_header(ctx, title, hint)

    if not issues:
        ctx.text(PADDING, HEADER_H + 20,
                 f"No {issue_state_filter} issues.", size=13, color=C["muted"])
        _draw_footer(ctx, hint)
        return

    # Scrolling math.
    list_y = HEADER_H + 6
    list_h = h - HEADER_H - FOOTER_H - 6
    visible_rows = max(1, int(list_h / ITEM_H))
    if selected < list_scroll:
        list_scroll = selected
    elif selected >= list_scroll + visible_rows:
        list_scroll = selected - visible_rows + 1

    for i in range(list_scroll, min(len(issues), list_scroll + visible_rows)):
        row_index = i - list_scroll
        y = list_y + row_index * ITEM_H
        is_sel = (i == selected)
        if is_sel:
            ctx.rect(0, y, w, ITEM_H, fill=C["sel_bg"])

        issue = issues[i]
        number = issue.get("number", 0)
        title_text = issue.get("title", "(no title)")
        state = issue.get("state", "OPEN")
        author = (issue.get("author") or {}).get("login", "")
        labels = issue.get("labels") or []

        # State dot.
        dot_color = C["green"] if state == "OPEN" else C["muted"]
        ctx.text(PADDING, y + 12, "\u25cf", size=14, color=dot_color)

        # #number
        num_text = f"#{number}"
        ctx.text(PADDING + 22, y + 14, num_text, size=12,
                 color=C["muted"], monospace=True)

        # Title — truncate to fit.
        title_x = PADDING + 22 + len(num_text) * 8 + 12
        # Reserve space for labels + author on the right.
        right_reserve = _label_strip_width(labels) + len(author) * 7 + 24
        max_title_w = max(40, w - title_x - right_reserve - PADDING)
        max_chars = max(8, int(max_title_w / 7.5))
        if len(title_text) > max_chars:
            title_text = title_text[: max_chars - 1] + "\u2026"
        title_color = C["text"] if is_sel else C["subtext"]
        ctx.text(title_x, y + 14, title_text, size=13, color=title_color)

        # Labels — right side, priority labels colored, others use github color
        label_x = w - PADDING - len(author) * 7 - 16
        # Render labels right-to-left.
        for lbl in reversed(labels[:4]):
            chip_text, chip_color = _label_chip(lbl)
            chip_w = len(chip_text) * 7 + 10
            label_x -= chip_w + 6
            ctx.rect(label_x, y + 8, chip_w, 20, fill=chip_color, radius=4.0)
            ctx.text(label_x + 5, y + 12, chip_text, size=11, color=C["bg"])

        # Author — far right.
        if author:
            author_x = w - PADDING - len(author) * 7 - 4
            ctx.text(author_x, y + 14, author, size=11, color=C["muted"])

    _draw_footer(ctx, hint)


def _label_chip(label: dict) -> tuple[str, str]:
    """Return (display_name, hex_color) for a label."""
    name = label.get("name", "")
    if name in PRIORITY_COLORS:
        return name, PRIORITY_COLORS[name]
    color_hex = label.get("color") or ""
    if color_hex and not color_hex.startswith("#"):
        color_hex = "#" + color_hex
    if not color_hex:
        color_hex = C["surface"]
    return name, color_hex


def _label_strip_width(labels: list) -> int:
    """Approximate width in px for the rendered label strip on the right."""
    if not labels:
        return 0
    total = 0
    for lbl in labels[:4]:
        name = lbl.get("name", "")
        total += len(name) * 7 + 16
    return total


def _render_detail(ctx):
    global detail_scroll
    w = ctx.width
    h = ctx.height

    # Find the issue header info from the cached list.
    cached = next((i for i in issues if i.get("number") == detail_issue_number), None)
    title_text = cached.get("title", f"#{detail_issue_number}") if cached else f"#{detail_issue_number}"

    title = f"#{detail_issue_number}  {title_text}"
    hint = "j/k scroll  Backspace back"
    _draw_header(ctx, title, hint)

    if not detail_data:
        ctx.text(PADDING, HEADER_H + 20, "Loading\u2026", size=13, color=C["muted"])
        return

    # Build a flat list of (color, text) lines for body + comments.
    char_width = max(30, int((ctx.width - 2 * PADDING) / 7.5))
    lines: list[tuple[str, str]] = []

    body = detail_data.get("body") or ""
    for line in wrap_text(body, char_width):
        lines.append((C["text"], line))

    comments = detail_data.get("comments") or []
    if comments:
        lines.append((C["text"], ""))
        lines.append((C["accent"], f"COMMENTS ({len(comments)})"))
        lines.append((C["surface"], "\u2500" * char_width))
        for c in comments:
            author = (c.get("author") or {}).get("login", "")
            created = (c.get("createdAt") or "")[:10]
            header = f"{author} \u00b7 {created}".strip(" \u00b7")
            lines.append((C["muted"], header))
            for line in wrap_text(c.get("body") or "", char_width):
                lines.append((C["text"], line))
            lines.append((C["text"], ""))

    body_top = HEADER_H + 12
    body_bottom = h - FOOTER_H - 4
    visible_lines = max(1, int((body_bottom - body_top) / LINE_H))
    max_scroll = max(0, len(lines) - visible_lines)
    detail_scroll = max(0, min(detail_scroll, max_scroll))

    for i, (color, text) in enumerate(lines[detail_scroll:detail_scroll + visible_lines]):
        y = body_top + i * LINE_H
        ctx.text(PADDING, y, text, size=13, color=color)

    # Scroll indicator.
    if len(lines) > visible_lines:
        pct = detail_scroll / max(1, max_scroll)
        indicator = f"{int(pct * 100)}%"
        ctx.text(w - PADDING - len(indicator) * 7, body_bottom - 4,
                 indicator, size=11, color=C["muted"])

    _draw_footer(ctx, hint)


def wrap_text(text: str, width: int) -> list[str]:
    """Wrap text to width characters, preserving paragraph breaks."""
    out: list[str] = []
    for para in text.split("\n"):
        if para.strip() == "":
            out.append("")
        else:
            wrapped = textwrap.wrap(para, width)
            out.extend(wrapped or [""])
    return out


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

# Kick off the preflight check immediately so the first render shows status.
begin_preflight()
app.run()
