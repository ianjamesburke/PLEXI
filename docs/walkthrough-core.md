Stint: 1401
Status: active

# Core walkthrough: seven steps on an installed alpha

Release 0.6 Palette. The steps are 1 to 6 and 12 of `.stint/runs/workbench-plan/PLAN.md` section 1 (ledger row 176). Keys come from `KeyBindings::default` in `src/host/keys.rs`, the Explorer's `EXPLORER_KEYS` in `src/file_browser/mod.rs` and the palette's `picker_key` in `src/ui/picker.rs`.

## Before you start

- An installed alpha, in a profile that has never run Plexi: a fresh macOS account, or no `~/.plexi-alpha`. Install with `curl -fsSL https://plexiapp.com/install | bash -s -- --channel alpha`, then open a new Plexi pane. Run the CLI as `plexi-alpha`, never bare `plexi`.
- Agent definitions need the `agent_mesh` experiment: `plexi-alpha experiments enable agent_mesh`. It is read at each use, no restart.
- One defined agent: `plexi-alpha agent create walker --cli claude --persona "You are a test agent."` (any harness you have installed). Use the same name in steps 1, 2 and 12.
- A workspace folder with a `README.md` in it. The terminal's working directory is that folder.
- Every key press is yours. The host ignores synthetic input on consent and destructive prompts. A press that lands too early on one reads "Wait a moment, then press again."

Record each step as pass, fail or blocked, with what you saw. A step passes only if every expected result happens.

## Steps

| # | Do | Expect |
|---|---|---|
| 1 | Cmd+P. Tab, Tab (chip reads Agents). Type `walker`. Enter. | The row shows `walker`, "not running". Enter opens a new chat beside the terminal. Close that pane, repeat: the last chat resumes. Repeat with the pane open: focus jumps to it. |
| 2 | In the agent pane, ask: "Ask me a question with two choices, then wait for my answer." Click the terminal pane. When the agent has asked, press Cmd+I. | Nothing took focus when the question arrived. The queue lists the question. Enter on it opens the answer card. Press 1 or 2 (or move with J, K or the arrows, then Enter). The agent continues with your choice. Esc closes the card and leaves the question queued. |
| 3 | Focus the terminal. Cmd+E. Type `rea`. Press Cmd+Shift+. . | Explorer covers the terminal as an overlay, not a split. The field reads `rea` and the list shows only matching names, in order. Cmd+Shift+. toggles hidden files. Record whether the choice survives opening another folder; this build does not remember it. |
| 4 | With `rea` still filtering, select `README.md` and press Enter. | The file opens in the editor. Record where it lands (beside, below or over the Explorer) and whether the text shows. |
| 5 | Close the Explorer. Cmd+P. Tab (chip reads Files). Type `readme`. | The list shows files under the workspace folder with a preview on the right, and no Explorer is open. Enter opens the file in the editor beside the focused pane. Tab again is Agents, a third Tab is Commands, Shift+Tab goes back. The query stays across stops. |
| 6 | Cmd+E. Cmd+N. Return to the original page and type `/new file t.md` in the Explorer. Press Enter. | Cmd+N opens a new terminal page to the right. The Explorer command creates `t.md` in the current folder and opens it in the editor. |
| 12 | Type some text in the open editor and leave it unsaved. Cmd+Q. | A dialog asks about the unsaved text. Choose Save, then confirm Plexi quits. |
| | Open Plexi again. | The layout returns: terminal, agent pane and editor where they were. |
| | Cmd+P, Tab, Tab, `walker`, Enter. | The same chat resumes, with its earlier messages. |
| | In a Plexi terminal: `plexi-alpha agent new-chat walker`. Then open `walker` as above. | The old chat is archived and a new, empty one opens. The memory folder is untouched: ask the agent for something you told it before. |

## Where a step depends on this build

- Step 2: the two-choice round trip has not been run on an installed build. If the queue shows the question but offers no choices, record that.
- Step 4: placement from an overlaid Explorer has not been checked. The Done-When asks for beside.
- Step 12: New chat is a CLI verb. The palette's Actions list for an agent holds Focus, Answer and Close pane. Restore of the layout after a quit has not been run on an installed build.

## Pass

All seven steps pass for a tester on a clean profile and again for Ian. Failures go to the stint of the step's area, not into this file.
