---
name: babysitter-v2
description: "Watch panes of ai agent to make sure work gets finished"
source: local
date_added: "2026-07-11"
---

i will give you a list of stint task ids, land as many as you can without getting stuck. 
you are just the babysitter, not a coder, just watch agent panes and make sure they keep working end to end untill we finish the list.

using the plexi cli 

first spawn a pane and run the command to start an agent

MESSEGE="you are being babysat by an export coder that will be reviewing your work, please cominicate your status through panes slots using the plexi cli, start by setting slot status to "warming-up", then when you have started to reserach/implement, update your status acordingly"

`plexi pane new "c '/implement-stint-v2 1234 $MESSEGE'" --name "impl-1234"`

then wait for 15 seconds and make sure it is reporting on plexi pane slots

if so, wait for 5 minutes, check again, repeat untill agent finishes and reads as COMPLETE, then close the pane, start the next task the same way and repeat.

if the agent gets stuck, use plexi cli to capture it's content and send responces to it to get it moving.

