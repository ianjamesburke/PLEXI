---
name: implement-stint-v2
description: "given a stint (task), implement it into a PR, as one of many commits."
risk: low
source: local
---

you maintain your state using the plexi cli

first see if alpha is clean, if not set state to blocked and send a plexi notification

  plexi notify --title "Error: Implemntenet stint 1234" \
    --body "stint XYZ failed because alpha branch is dirty." \
    --choice "Go to Context:pane_focus:$PLEXI_PANE_ID" \
    --scope context 


if clean, `stint claim <task-id>` to claim on alpha, marking as in progress

report status:
plexi pane slot write status
      "warming-up" --replace; plexi pane slot
      write pipeline_phase
      "implement-stint-v2:0728" --replace


name the conversation:
i.e 'plexi pane name $PLEXI_PANE_ID impl-1234-sidebar-padding-fix'




then create a worktree with 'wtp add -b <stint-is-number>-impl-<stint title/short desc>

ALL WORK LIVES IN THIS WORKTREE

you only need to run test relevent to the code change, not the full suite. 

if stint is 's' - small
- immediatly look for the code change and implement swiftly, run e2e cargo test and finish

if m
Have a subagent generate a plan by reading the stint description, looking at the relevant code, and creating an execution pla
then close that subagent and send the exicution plan to a new subagent. 

if l
have one subagent do a reasearch pass to create a multiphase plan
then spawn subagents in this order. 
impl-phase-1 - Only handling the implementation of the code for phase one.
test-phase-2 - only responible for testing and reviewing the code quality, and running host tests
impl-phase-1-b - If test phase 2 comes back with errors or suggestions on code quality changes as new commits on the worktree, 

repeat impl and test loop untill all work in the plan lands

during testing, try to use the plexi scens test to create screenshots if you can fo better verification

---

Output on successful implementation:

open a pr against alpha and wait for CI CD to pass
then run just pr-install the pr number

and send this to the user.

```text
[IMPLEMENTED]
Stint <task-id>
Branch: feature/stint-<task-id>-<slug>
Files changed: <N>

TESTING:
<a short list of commands to run or things to spot check>
<e.g. 'Run 'plexi-pr-1223 app list and make sure that... >


"Pass, follow up, or fail?" 

```

if:
PASS - Spawning sub-agent to merge pr into alpha, rebasing if needed, Addressing any merge conflicts, running just clean up commands. finally mark stint complete at the end, then say COMPLETE
FOLLOW-UP - a list of improvments that need to be made, have a subagent go implemment those chages on the same worktree, commit to the same pr, and reinstall the new version with just pr-install. continue this till PASS
FAIL - Add a section on the stint file that explains the fail, List some possible gotchas if there are any, as well as the user's feedback for the next attempt at implementation. mark stint as todo again. 

During cleanup, move to alpha because issues arise if you try to remove a work tree you're currently working from. 