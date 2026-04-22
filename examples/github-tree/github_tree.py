#!/usr/bin/env python3
"""GitHub Tree — browse repos, branches, and commits via the GitHub API."""
from __future__ import annotations

import json
import queue
import threading
import urllib.parse
import uuid

from plexi_sdk import (
    App, RenderContext,
    BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT, RED, GREEN, YELLOW,
    BODY, CAPTION, HINT, HEADING,
    PAD, PAD_TIGHT, HEADER_H,
    dim, _emit,
)

GITHUB_API = "https://api.github.com"

MODE_SEARCH  = "search"
MODE_LOADING = "loading"
MODE_REPO    = "repo"
MODE_ERROR   = "error"

SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]


def _trunc(s: str, max_chars: int) -> str:
    if len(s) <= max_chars:
        return s
    return s[: max_chars - 1] + "…"


class GitHubTreeApp(App):

    # ── Lifecycle ─────────────────────────────────────────────────────────────

    def on_init(self, ctx: RenderContext) -> None:
        self._mode = MODE_SEARCH
        self._query = ""
        self._token: str | None = None

        self._repo_info: dict = {}
        self._branches: list[str] = []
        self._commits: list[dict] = []

        self._branch_sel = 0
        self._spinner_frame = 0
        self._error = ""
        self._loading_msg = ""

        threading.Thread(target=self._load_secret, daemon=True).start()

    def _load_secret(self) -> None:
        try:
            value = self.emit.get_secret("HOMEBREW_TAP_TOKEN")
            if value:
                self._token = value
                self.emit.debug("github-tree: auth token loaded")
        except Exception as e:
            self.emit.warn(f"github-tree: secret fetch failed: {e}")

    # ── Authenticated HTTP (with custom headers) ──────────────────────────────

    def _github_get(self, url: str) -> dict | list:
        """Blocking authenticated GET to api.github.com. Returns parsed JSON."""
        req_id = str(uuid.uuid4())
        q: queue.Queue = queue.Queue()
        self._pending_http[req_id] = q

        headers = {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        if self._token:
            headers["Authorization"] = f"Bearer {self._token}"

        _emit({
            "type": "http_request",
            "request_id": req_id,
            "method": "GET",
            "url": url,
            "headers": headers,
        })

        status, value = q.get()
        if status == "error":
            raise RuntimeError(value)
        return json.loads(value)

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._mode == MODE_SEARCH:
            self._handle_search_key(key)
        elif self._mode == MODE_REPO:
            self._handle_repo_key(key)
        elif self._mode == MODE_ERROR:
            if key in ("escape", "return"):
                self._mode = MODE_SEARCH
        # Loading: Escape returns to search
        elif self._mode == MODE_LOADING and key == "escape":
            self._mode = MODE_SEARCH

    def _handle_search_key(self, key: str) -> None:
        if key == "return":
            slug = self._query.strip()
            if "/" in slug:
                self._fetch_repo(slug)
        elif key == "backspace":
            self._query = self._query[:-1]
        elif key == "space":
            self._query += " "
        elif len(key) == 1:
            self._query += key

    def _handle_repo_key(self, key: str) -> None:
        if key == "escape":
            self._mode = MODE_SEARCH
        elif key in ("j", "down"):
            self._branch_sel = min(len(self._branches) - 1, self._branch_sel + 1)
        elif key in ("k", "up"):
            self._branch_sel = max(0, self._branch_sel - 1)
        elif key == "return":
            if self._branches:
                branch = self._branches[self._branch_sel]
                slug = self._repo_info.get("full_name", "")
                if slug:
                    self._fetch_commits(slug, branch)
        elif key == "r":
            slug = self._repo_info.get("full_name", "")
            if slug:
                self._fetch_repo(slug)

    # ── Networking ────────────────────────────────────────────────────────────

    def _fetch_repo(self, slug: str) -> None:
        self._mode = MODE_LOADING
        self._loading_msg = f"Fetching {slug}…"
        self._error = ""

        def run() -> None:
            try:
                owner, repo = slug.split("/", 1)
                info      = self._github_get(f"{GITHUB_API}/repos/{owner}/{repo}")
                branches  = self._github_get(f"{GITHUB_API}/repos/{owner}/{repo}/branches?per_page=30")
                commits   = self._github_get(f"{GITHUB_API}/repos/{owner}/{repo}/commits?per_page=5")

                self._repo_info  = info  # type: ignore[assignment]
                self._branches   = [b["name"] for b in branches]  # type: ignore[union-attr]
                self._commits    = _parse_commits(commits)  # type: ignore[arg-type]
                self._branch_sel = 0
                self._mode       = MODE_REPO
                self.emit.status_summary(info.get("full_name", slug))  # type: ignore[union-attr]
            except Exception as e:
                self._error = str(e)
                self._mode  = MODE_ERROR
                self.emit.status_summary("Error")

        threading.Thread(target=run, daemon=True).start()

    def _fetch_commits(self, slug: str, branch: str) -> None:
        self._loading_msg = f"Commits for {branch}…"
        self._mode = MODE_LOADING

        def run() -> None:
            try:
                commits = self._github_get(
                    f"{GITHUB_API}/repos/{slug}/commits"
                    f"?sha={urllib.parse.quote(branch)}&per_page=5"
                )
                self._commits = _parse_commits(commits)  # type: ignore[arg-type]
                self._mode    = MODE_REPO
            except Exception as e:
                self._error = str(e)
                self._mode  = MODE_ERROR

        threading.Thread(target=run, daemon=True).start()

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        self._spinner_frame = (self._spinner_frame + 1) % len(SPINNER)
        ctx.clear(BG)
        _draw_header(ctx, self._token is not None)

        if self._mode == MODE_SEARCH:
            _draw_search(ctx, self._query)
        elif self._mode == MODE_LOADING:
            _draw_loading(ctx, SPINNER[self._spinner_frame], self._loading_msg)
            ctx.emit.schedule_render(after_ms=80)
        elif self._mode == MODE_REPO:
            _draw_repo(ctx, self._repo_info, self._branches,
                       self._branch_sel, self._commits)
        elif self._mode == MODE_ERROR:
            _draw_error(ctx, self._error)


# ── Stateless draw helpers ────────────────────────────────────────────────────

def _draw_header(ctx: RenderContext, authed: bool) -> None:
    ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
    ctx.text(PAD, 14, "GitHub Tree", size=HEADING, color=ACCENT, bold=True)
    dot_color = GREEN if authed else YELLOW
    dot_label = "auth" if authed else "anon"
    ctx.circle(ctx.w - PAD - 4, 24, 5, dot_color)
    ctx.text(ctx.w - PAD - (10 if authed else 14) - 18, 16,
             dot_label, size=HINT, color=MUTED)


def _draw_search(ctx: RenderContext, query: str) -> None:
    top = HEADER_H + 48
    ctx.text(PAD, top, "Open a GitHub repository", size=HEADING, color=FG, bold=True)
    ctx.text(PAD, top + 26, "owner/repo", size=CAPTION, color=MUTED)

    box_y = top + 60
    ctx.rect(PAD, box_y, ctx.w - PAD * 2, 44, fill=SURFACE, radius=8.0)
    ctx.rect(PAD, box_y, ctx.w - PAD * 2, 44, fill=dim(ACCENT, 25), radius=8.0)

    max_chars = max(1, int((ctx.w - PAD * 2 - PAD_TIGHT * 2) / 8))
    display = _trunc(query, max_chars - 1) + "▌"
    ctx.text(PAD + PAD_TIGHT, box_y + 13, display, size=BODY, color=FG, monospace=True)

    ctx.text(PAD, box_y + 60, "Return to fetch  ·  Esc to clear",
             size=HINT, color=MUTED)
    ctx.text(PAD, box_y + 80, "e.g.  anthropics/anthropic-sdk-python",
             size=HINT, color=dim(MUTED, 140))


def _draw_loading(ctx: RenderContext, spinner: str, msg: str) -> None:
    cx = ctx.w / 2
    cy = ctx.h / 2
    label = f"{spinner}  {msg}"
    # Rough centering: 8px per char at BODY size
    lx = cx - len(label) * 4
    ctx.text(lx, cy - 10, label, size=BODY, color=ACCENT)


def _draw_repo(ctx: RenderContext, info: dict, branches: list[str],
               branch_sel: int, commits: list[dict]) -> None:
    w = ctx.w

    # ── Info card ─────────────────────────────────────────────────────────────
    cx = PAD
    cy = HEADER_H + PAD
    cw = w - PAD * 2
    ch = 84
    ctx.rect(cx, cy, cw, ch, fill=SURFACE, radius=8.0)

    name        = info.get("full_name", "unknown")
    description = info.get("description") or ""
    stars       = info.get("stargazers_count", 0)
    forks       = info.get("forks_count", 0)
    language    = info.get("language") or ""

    ctx.text(cx + PAD, cy + 10, _trunc(name, 42), size=HEADING, color=ACCENT, bold=True)
    max_desc = max(1, int((cw - PAD * 2) / 7))
    ctx.text(cx + PAD, cy + 34, _trunc(description, max_desc), size=CAPTION, color=FG)

    meta_parts = []
    if stars:
        meta_parts.append(f"★ {stars:,}")
    if forks:
        meta_parts.append(f"⑂ {forks:,}")
    if language:
        meta_parts.append(language)
    ctx.text(cx + PAD, cy + 56, "  ·  ".join(meta_parts), size=HINT, color=MUTED)

    # ── Columns ───────────────────────────────────────────────────────────────
    section_y  = cy + ch + PAD
    branch_col = int(w * 0.35)
    commit_col = w - branch_col - PAD * 2 - PAD_TIGHT
    commit_x   = PAD + branch_col + PAD_TIGHT

    # Branches column
    ctx.text(PAD, section_y, "Branches", size=CAPTION, color=MUTED, bold=True)
    ctx.line(PAD, section_y + 18, float(PAD + branch_col),
             section_y + 18, color=HIGHLIGHT)

    list_y = section_y + 22
    list_h = ctx.h - list_y - 32
    branch_items = [{"title": b} for b in branches]
    ctx.list(branch_items, selected=branch_sel,
             item_height=32.0, x=PAD, y=list_y,
             w=float(branch_col), h=float(list_h))

    # Commits column
    ctx.text(commit_x, section_y, "Recent Commits", size=CAPTION, color=MUTED, bold=True)
    ctx.line(commit_x, section_y + 18, commit_x + commit_col,
             section_y + 18, color=HIGHLIGHT)

    ry = section_y + 22
    max_msg = max(1, int((commit_col - PAD_TIGHT * 2) / 7))
    for c in commits:
        if ry + 56 > ctx.h - 32:
            break
        ctx.rect(commit_x, ry, commit_col, 52, fill=SURFACE, radius=6.0)
        ctx.text(commit_x + PAD_TIGHT, ry + 7,
                 f"[{c['sha']}]", size=HINT, color=ACCENT, monospace=True)
        ctx.text(commit_x + PAD_TIGHT + 60, ry + 7,
                 _trunc(c["author"], 20), size=HINT, color=MUTED)
        ctx.text(commit_x + PAD_TIGHT, ry + 27,
                 _trunc(c["message"], max_msg), size=CAPTION, color=FG)
        ry += 58

    # Footer
    ctx.text(PAD, ctx.h - 22,
             "j/k branches  ·  Return load commits  ·  r refresh  ·  Esc search",
             size=HINT, color=MUTED)


def _draw_error(ctx: RenderContext, error: str) -> None:
    cy = ctx.h / 2
    ctx.rect(PAD, cy - 44, ctx.w - PAD * 2, 88, fill=SURFACE, radius=8.0)
    ctx.text(PAD + PAD_TIGHT, cy - 28, "Error", size=HEADING, color=RED, bold=True)
    max_chars = max(1, int((ctx.w - PAD * 2 - PAD_TIGHT * 2) / 7))
    ctx.text(PAD + PAD_TIGHT, cy, _trunc(error, max_chars), size=CAPTION, color=FG)
    ctx.text(PAD + PAD_TIGHT, cy + 24, "Escape to go back", size=HINT, color=MUTED)


# ── Helpers ───────────────────────────────────────────────────────────────────

def _parse_commits(raw: list) -> list[dict]:
    out = []
    for c in raw[:5]:
        commit  = c.get("commit", {})
        sha     = c.get("sha", "")[:7]
        message = commit.get("message", "").split("\n")[0]
        author  = commit.get("author", {}).get("name", "?")
        out.append({"sha": sha, "message": message, "author": author})
    return out


if __name__ == "__main__":
    GitHubTreeApp().run()
