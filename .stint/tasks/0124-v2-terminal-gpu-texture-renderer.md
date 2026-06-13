---
id: "0124"
title: "v2 terminal: offscreen wgpu texture renderer"
status: backlog
sprint: "s28"
estimate: 12h
blocked_by: []
gh_issue: ["2068"]
area: ["egui_term"]
tags: ["v2", "terminal", "renderer"]
---

Render terminal glyphs into a dedicated offscreen wgpu texture to address clipping and unlock better shaping, emoji, and antialiasing.
