# Human Checks — pending validations only a human can do

Appended by the babysitter loop at merge time for stints with a `## Human Check` section (see SKILL.md, "Human-check queue"). Check items off (`- [x]`) as you do them; checked items are pruned at the next run's start. Evidence paths are stable under `.stint/evidence/`.

- [ ] Stint 0358 (production hosted-registry install smoke) SKIPPED by the overnight run, untouched, still todo. It is not agent-work: it requires deploying current alpha to PRODUCTION and then doing a real install from a clean machine/profile against the production registry. Deploying production unattended was outside what the run was authorized to do. It is the last unverified step on the free-v1 finish line, so it wants a deliberate slot with you driving.
- [ ] **FAILED 2026-08-01 by Ian — see stint 0693.** The cue is unreachable: `plexi notify` hardcodes priority=50, default interrupt_threshold is 100, so `cue=false` always. Recheck only after 0693 lands. Original entry: PR #2506 (0566: notification choke point + audible cue) MERGED. **THE AUDIBLE CUE WAS NEVER ONCE EMITTED — deliberately, because you were asleep in the same room as this machine all night.** So the one thing 0566 exists to do is the one thing no agent verified. Do this: set `[notifications] sound = "/System/Library/Sounds/Ping.aiff"` in your config, trigger a notification that is actually allowed to interrupt (visible context, focus mode off, priority at or above `interrupt_threshold` — a below-threshold or background-context notification is now *correctly* silent, so testing with one will look like a bug and is not), and confirm you hear it **once**, not clipped short and not repeated. Then confirm the inverse: with `focus_mode = true`, or with the sending context in the background, you hear **nothing**. Judging: is the cue pleasant and correctly timed, and does silence happen exactly when it should. WHY THIS MERGED UNHEARD: both static-review blockers were fixed and I verified each in the code myself; the cue's gating is proven by unit tests that **fail** when the predicate is reverted; and a live tester round measured the whole notification-routing surface with `cue=true count: 0` across the entire session. Note `sound` ships commented out in `scripts/default-config.toml`, so nobody gets an unexpected noise until you opt in. Evidence: PR #2506 body; tester report is transient.
- [ ] **REVIEWED 2026-08-01 by Ian — no leak concern, content verdict TOO WORDY. Rewrite filed as stint 0695; re-read after that lands.** Original entry: PR #2507 (0570: publish the plexi-cli skill via npx skills) MERGED — and it created a **permanent public repo under your name**: `https://github.com/ianjamesburke/plexi-skills`, installable right now by anyone with `npx skills add ianjamesburke/plexi-skills`. **No leak: I audited it on every push** (tree = exactly `README.md` + `skills/plexi-cli/SKILL.md`; two commits, both touching only those; content scanned for machine/personal markers — nothing from `babysitter/`, `LOG.md`, `RUN_STATE.md`, or any internal workflow skill). It still wants one human read, because it is public and permanent and no agent can judge how it reads to a stranger: skim the README and SKILL.md once as if you were a developer who has only the `plexi` binary. Judging: does this represent the project the way you want it to, publicly and indefinitely. **The one residual worth knowing:** the mirror was **hand-corrected** back to the v0.2.0 surface rather than produced by the new release flow — the `v0.2.0` tag has no `skills/` dir at all, since this PR creates it — so the release gate never validated *this particular* published commit. A tester independently verified it against a real v0.2.0 binary (0 errors), and from the next release onward `promote.sh` closes the loop automatically. Nothing to do about it; just know that the first published copy is the one commit the machine never checked end to end.

- [ ] PR #2546 (0674: todo rebuilt on SDK primitives): the todo WRITE tools (todo.add / set_done / remove) invoked from
  the Assistant hit the ask-gate permission dialog, which is out of tester scope by your 2026-08-01 ruling, so no agent
  verified them. Drive it by hand: open todo, ask the assistant to add an item, complete it, and remove it, approving each
  prompt. Judging: do the writes land correctly and does the UI reflect them immediately. Note the read-only tools were
  verified working while the pane is on screen. A SEPARATE off-screen timeout defect was found and traced to a
  pre-existing host defect that reproduces on plain alpha with an app predating this PR, so it was dropped from this
  PR's gate and filed as stint 0688 — every app tool call to a pane that is not rendering times out, todo included.
  Until 0688 lands, drive this check with the todo pane on screen. Evidence: tester report is transient; PR #2546 body.

                                                                              
  [ ] **PR #2561 (stint 0711: delete the notification priority system) —      
  TESTER PASS, MERGE HELD FOR THIS CHECK.**                                   
  This supersedes the 0566/0693 entry above: plexi notify no longer has a     
  priority number, a --level flag, or an                                      
  interrupt_threshold to clear. One level — every visible, non-muted          
  notification now interrupts and cues, so the                                
  "test with a below-threshold notification and it looks broken" trap is gone 
  with it.                                                                    
  Do this: plexi-pr-2561 is still installed. Set [notifications] sound =      
  "/System/Library/Sounds/Ping.aiff" in                                       
  ~/.plexi-pr-2561/config.toml, run a plain plexi-pr-2561 notify with a title 
  and body, and confirm you hear it                                           
  **once** — not clipped, not repeated. Then set focus_mode = true and fire   
  the same notification: you must hear                                        
  **nothing**. Judging: is the cue pleasant and correctly timed, and does     
  silence happen exactly when it should.                                      
  No agent has ever heard this cue; the overnight rounds deliberately never   
  emit sound.                                                                 
  What the agents DID verify: a plain CLI notify now logs interrupt=true (it  
  logged interrupt=false on alpha);                                           
  an app-wire request carrying priority is rejected live with unknown field   
  'priority' rather than silently                                             
  ignored; interrupt_threshold is gone from config; both new guards were      
  falsified in a scratch tree and failed                                      
  under deliberately restored bad policies; no priority / --level /           
  interrupt_threshold residue survives in                                     
  docs, CLI help, or the SDK surface. Evidence: PR #2561 body; tester report  
  is transient.                                                               

