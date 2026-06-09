## .agents

This is the master skills directory. All Plexi skills live in `skills/` here (as symlinks to `../../skills/<name>/`) and are the canonical source for any agent that needs them.

If an agent runtime doesn't look here natively (e.g. Claude Code reads from `~/.claude/skills/`), symlink from that runtime's discovery path back to `.agents/skills/<name>`. Don't copy files. One source of truth, everything else points here.

Chain: `~/.claude/skills/<name>` -> `.agents/skills/<name>` -> `../../skills/<name>` (the actual files).
