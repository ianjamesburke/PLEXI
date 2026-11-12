---
title: Quick Note
description: Capture a note from anywhere in Plexi with one shortcut.
order: 7
---

Quick Note is a small compose box that opens over whatever you are doing. Type
a thought, press `Enter`, and you are back where you were with the note saved.

## Capture a note

Press `⌘0` from anywhere inside Plexi. Type the note.

| Action | Key |
|--------|-----|
| Save the note and close | `Enter` |
| New line | `Shift+Enter` |
| Close without saving | `Escape` |

An empty note is never saved. Text you dismiss with `Escape` is discarded.

## Where notes go

Plexi saves each note as a timestamped Markdown file in the notes folder of
the context you are in. A note records the working directory of the pane you
captured it from, so you can tell later what you were looking at.

The same notes are available from the CLI:

```sh
plexi note "remember to update the docs"   # capture without the overlay
plexi note list                            # print note paths, newest first
plexi note show <path>                     # print one note
plexi note open                            # pick a note with fzf
plexi note rm <path>                       # delete one
```

`plexi note list` covers the current context, any context nested below it, and
your global notes. Agents use these same commands, so a note you capture is
something an agent can read when you point it there.

## Change the shortcut

Rebind `open_quick_note` under `[keys]` in `config.toml`. See
[Configuration](/docs/config).
