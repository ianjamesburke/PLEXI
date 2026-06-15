---
id: "0153"
title: "Syntax highlighting primitive and ReadOnlyCodeViewer widget"
status: done
estimate: "12h"
actual: "2h"
started_at: "2026-06-15T07:55:27Z"
completed_at: "2026-06-15T17:44:37Z"
blocked_by: []
gh_issue:
  - "2168"
area:
  - "ui/widgets"
  - "apps/github-issues"
tags:
  - "markdown"
  - "syntax"
  - "text-editor"
  - "tree-sitter"
---





Build the shared syntax highlighting foundation:

1. Add `inkjet` (tree-sitter wrapper) to `Cargo.toml`
2. `src/ui/syntax.rs` — `SyntaxHighlighter::highlight(code, lang, colors) -> LayoutJob` with token-type → PlexiColors table
3. `src/ui/code_viewer.rs` — `ReadOnlyCodeViewer` egui widget (scrollable, monospace, optional line numbers)
4. Wire into `RenderCommand::Markdown` to replace egui_commonmark's plain monospace code blocks
5. POC example app or inline demo with Rust + TOML + Python blocks

This is the foundation for the text editor — the same `SyntaxHighlighter` feeds editor highlighting when that work begins.

First step: resolve the open question in #2168 about whether egui_commonmark exposes a code block hook or whether we need to parse fenced blocks ourselves before passing body text to egui_commonmark.

## Implementation Note

Current alpha already had fenced-code highlighting through the egui markdown stack, so this pass added the shared `SyntaxHighlighter` and `ReadOnlyCodeViewer` primitives without replacing markdown rendering or adding `inkjet`.

## Variance

Estimate assumed a new tree-sitter/markdown hook path. The shipped slice reused `egui_extras` syntax highlighting already aligned with egui and proved the primitive in Host UI Gallery.
