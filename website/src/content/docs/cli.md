---
title: CLI Overview
description: The plexi command-line interface — all subcommands at a glance.
verified_version: "3.6.19"
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and apps from outside the UI.

All commands work identically across build channels (`plexi`, `plexi-alpha`, `plexi-beta`). When run inside a Plexi pane, `PLEXI_SOCKET` routes host commands to the correct running instance automatically.

## Pane Commands

```sh
plexi pane name "my pane"    # rename the focused pane
plexi pane close             # close the focused pane
```

## Terminal

Open a new terminal pane and optionally run a command:

```sh
plexi terminal                       # open a blank terminal
plexi terminal -- npm run dev        # open terminal, run command, keep shell alive
```

## Quick Note

```sh
plexi note                   # open Quick Note
```

## Notifications

Send a push notification to the user:

```sh
plexi notify "Build complete" --body "npm run build finished in 4.2s"
```

## Context

Read or update workspace context values:

```sh
plexi context get git_branch
plexi context set my_key my_value
```

## Workspace

Initialize or inspect a workspace in the current directory:

```sh
plexi workspace init         # create .plexi/workspace.toml
plexi workspace status       # show workspace info
```

## Apps

```sh
plexi app init my-app        # scaffold a new app
plexi app run my-app         # run app in focused pane
plexi app list               # list apps in the current workspace
```

## Secrets

```sh
plexi secrets set KEY value
plexi secrets get KEY
plexi secrets list
plexi secrets delete KEY
```

## Shell Completions

Generate completions for your shell:

```sh
plexi completions zsh        # output zsh completions
plexi completions bash
plexi completions fish
```
