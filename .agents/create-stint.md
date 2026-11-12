# Plexi stint creation rules

Apply these rules with the global `create-stint` skill.

- Read the relevant PRM and `TRUTH.md` before scoping work. Stint is the operating graph; do not maintain a separate priority projection.
- Plan qualification against `src/testing/TESTING.md`: affected behavior and risk, checks, scene or visual review, installed boundary, and evidence. Record the provisional plan in `## Qualification`; reassess it against the actual implementation diff.
- Use a flat task list. Do not use sprint commands or add sprint fields. Order work with priority and real `blocked_by` dependencies.
- Set priority, size, at least one existing area, and at least one of the `v1` or `v2` tags. Size means effort, not validation risk. Do not add a `time_estimate` field.
- Set exactly one risk tag: `risk-low`, `risk-normal` or `risk-high`. Risk decides the landing path; size stays effort, because a one-line broker change can be high risk.
  - `risk-low`: docs, copy, config text, or a single-file low-risk fix with no host UI. Commit directly to `alpha` after local tests and gates. No PR, no independent tester.
  - `risk-normal`: a PR and a candidate build, with the implementer's own tests and screenshots. Merge with Ian's OK after he has looked, or his pre-authorization. No independent tester.
  - `risk-high`: security, permissions, input handling, deleting user files, the terminal core, release or install, or any surface an agent invokes. A PR, a candidate build, and an independent tester on the installed exact head before merge.
  - With no risk tag, treat the stint as `risk-normal`. Ian can override the tier on any stint.
- Use `## Done-When` for observable acceptance criteria; babysitter-v3 requires this heading. Include `## Why` with a line explaining why the scope is the proper fix, and references to relevant files. Omit empty optional sections.
- Do not create a GitHub issue unless explicitly requested. Check existing issues for overlap as well as existing stints. When a proposed task supersedes an existing issue, follow the global skill and the user's authorization for external actions.
- After the batch, run `stint check` and verify each new task in `stint list`, including its priority, size, and status. Apply the root `AGENTS.md` rule for `TRUTH.md` updates; task creation alone does not require one.
- End with a plain confirmation for each successfully filed task: **Created stint <ID>: <title>**, its path, and the issue URL if one was created. Recommend either dispatching via `/implement-stint-v2 <ID>` or parking it for `stint next`.
