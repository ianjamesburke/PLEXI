## .agents

This is the master skills directory for **repo-internal workflow skills** (ship
pipeline, babysitter, and friends). All of them live in `.agents/skills/<name>/`
and are the canonical source for any agent that needs them.

If an agent runtime doesn't look here natively (e.g. Claude Code reads from
`~/.claude/skills/`), symlink from that runtime's discovery path back to
`.agents/skills/<name>`. Don't copy files. One source of truth, everything else
points here.

Chain: `~/.claude/skills/<name>` -> `.agents/skills/<name>`.

The user-facing **Plexi CLI skill is not here.** It lives at
`skills/plexi-cli/` and is installed per agent and per channel by
`plexi skill install` (see `skills/AGENTS.md`). Symlinking it under
`.agents/skills/` put a second, independently-stale `plexi-cli` on the same
agent's discovery path as the installed copy, and the two could disagree on
version and on which channel's surface they documented (stint 1050). There is
one discovery path: the installed skill.
