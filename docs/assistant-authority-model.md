# Assistant Authority Model

Status: active.
Stint: `0554`.
Parent: [`assistant-host-app.md`](assistant-host-app.md).
Last updated: 2026-07-27.

This is the authoritative threat model and capability design for the host Assistant. [`assistant-host-app.md`](assistant-host-app.md) owns the Assistant's product surface — agents, skills, slash commands, settings, UI, marketplace. This document owns the authority plane underneath it: what the Assistant is trusted to do, who decides, what a grant binds to, and what must be true before any new Assistant power ships.

The third-party app security model is not restated here. Capability declaration, manifest parsing, the wasmtime component boundary, and marketplace trust labels are owned by [`app-framework-marketplace.md`](app-framework-marketplace.md) and [`wasm-runtime.md`](wasm-runtime.md). This document describes only the boundary those documents do not cover: the host protecting itself from its own built-in Assistant.

## Threat Model

### What we are defending

The Assistant runs in-process inside the native Plexi host, as the logged-in macOS user. The host is not App-Sandboxed and not containerized. Everything that user can reach, a mistake in the Assistant's authorization path can reach: the entire home directory, every workspace and context, every running process, every terminal's environment, and the macOS Keychain entries Plexi itself can unlock.

Wasmtime bounds third-party apps. It does not bound the Assistant, because the Assistant is host code, and because the powers it needs — read this repository, run this project's test command, edit this file — are exactly the powers a sandbox would have to grant back.

### The adversary

The adversary is not primarily a malicious user. It is **untrusted text reaching a capable planner**. A remote model's output is a proposal, and its input is attacker-reachable: repository contents, a README, a web page a connector fetched, a terminal's scrollback, an app's semantic state, a note someone shared, an app manifest's tool descriptions. Prompt injection is the expected case, not the exotic one.

Secondary adversaries, in descending likelihood:

- A third-party app that lies in its own tool metadata to obtain silent invocation.
- A user who approves one narrow action and does not realize the approval generalized.
- A model that is simply wrong — mis-resolving a path, re-running a command with mutated arguments, addressing the wrong pane.

### Trust boundaries

- **The model is untrusted.** It proposes typed actions. It never holds authority and never carries a grant.
- **A context root is a relevance boundary, not a security boundary.** It selects what is in view and what is default-visible. It grants nothing. The separately-shipped caller-context workspace binding improves routing and must never be cited as isolation.
- **The host reference monitor is the only trusted authorizer.** It is the single choke point where an actor, an action, a resource, and a concrete argument set are checked together before any adapter executes.
- **App-declared metadata is untrusted input to policy, never policy.** A connector's schema, description, and mutability claim are hints for the model and for the UI. Host policy decides what a tool may do.
- **The user is the only source of new authority**, expressed through a permission decision that names the exact thing being approved.

### Assets, ranked

1. Workspace secrets and Keychain-resolvable values.
2. The filesystem outside the approved project root, especially the user's home directory and other contexts' roots.
3. Arbitrary process execution as the logged-in user.
4. Other contexts' panes, terminals, and app state — both reading them and controlling them.
5. Content leaving the machine in a model request.
6. The integrity of the audit trail itself.

### Known ambient-authority paths

These exist today and are the reason this document exists. Each is stated as the property that must become true, so the entry stays meaningful after the code changes.

**Grants do not bind to resources.** `GrantRecord` carries `resource_scope` and `resource_id`, and `GrantRecord::matches` never compares them. Matching is actor plus `target_id` plus workspace root. `PermissionRequest` has no field for arguments at all. The practical result: approving one file write approves every future file write in that workspace, and approving one terminal command approves every future command. The Assistant's session-approval set and ask-list are likewise keyed by bare tool name. Narrow binding is documented in the product spec and structurally present in the record type, but inert at evaluation time.

**Read-only is self-asserted and buys silent invocation.** A connector tool's `read_only` flag comes straight from the app's own declaration, and `AssistantApp::gated_dispatcher` auto-allows a read-only tool under an `Ask` decision with no prompt. An app that marks a mutating tool read-only gets unprompted execution. Grants for connector tools bind to a tool-name string rather than to app package identity, so two apps claiming the same tool name are indistinguishable to the grant store.

**Pane and terminal observation and control cross contexts.** `assistant_pane_list` walks every window in every context and takes no origin context parameter. `assistant_pane_state` and `assistant_read_terminal` resolve a pane id against all windows. Focus and close do the same. Terminal reads are marked read-only, so they are auto-allowed — any context's terminal scrollback, including any secret echoed into it, is readable without a prompt. The mutating terminal paths (`assistant_run_terminal`, `assistant_bind_terminal`) are context-checked and are the correct model; the read and control paths are not.

**The global apps directory is an unconditional writable root.** `assistant_file_roots` returns the caller's context root plus the installed-apps directory for every conversation in every workspace.

**Write plus build is unsandboxed execution.** `host.build.run` restricts argv to the `plexi app` init and check shapes, but checking an app spawns that app's interpreter. Combined with a workspace-scoped write grant, two approvals compose into arbitrary code execution as the user, outside any sandbox.

**The built-in bypass makes Assistant authority unenumerable.** `AppPermissions::builtin()` carries an empty capability set because `is_builtin` short-circuits the PGAP checks. The terminal-binding capability check is therefore vacuous for the Assistant. Its real restraints live only in the newer broker layer, and nothing lists what it actually holds or lets a user revoke it.

**Secrets can reach the transcript through auto-allowed reads.** Terminal reads, file reads, and grep are all read-only and therefore unprompted, and the walk skip-list excludes dependency and VCS directories but not dotfiles or `.env`. Build output returns raw subprocess stdout. There is no secret target type in the broker at all, so a policy line denying secret reads names a target that does not exist.

**The audit trail records intent, not outcome.** The high-risk tools write a "requested" event before execution and no completion record. The actor field is a hardcoded literal rather than the agent that ran.

## Design

### The reference monitor

One host-owned component authorizes every Assistant action. It is not advisory, not a lint, and not a prompt-level instruction. No adapter — file, process, pane, connector, network, secret — executes without a decision from it, and the decision is made against the fully-resolved action, not the intent.

An authorization request names:

- **Actor**: agent identity and scope, the delegating parent when the actor is a sub-agent, and the model backend that produced the proposal.
- **Action**: the stable capability id.
- **Resource identity**: the narrowest durable identity that exists for this action — context id, pane id, canonical filesystem path, connector plus app package identity, secret canonical name, or exact command shape.
- **Arguments**: the concrete, already-resolved argument set the adapter will receive.
- **Environment**: working directory and the set of injected secret names.
- **Origin**: the pane and context the request came from, supplied by the host and never by the model.

Two rules make this real. **Resolve before you authorize**: the monitor sees the canonicalized path, the expanded command, the resolved pane, never the model's raw string. **Authorize the same value you execute**: the adapter receives the exact authorized argument set as a token, so nothing can be re-read from ambient state between decision and execution. This is the general form of the existing repo-wide rule that a command handler's data must be self-contained.

Grant matching compares every identity field, including resource and arguments. A grant whose resource does not match the request does not match the request. Argument-bearing actions bind to a normalized argument fingerprint, so a changed argument is a new decision. Session-scoped approval binds the same fields as a persisted grant; it differs only in duration.

Every decision — deny, ask, allow, use, revoke — writes an audit record naming the actor, the resource, the arguments, and the grant that authorized it. Risk-bearing actions write both a request record and a completion record carrying the outcome. A user can answer "what did the Assistant actually do, under whose approval" from the audit log alone, without reading a transcript.

### Context isolation

The Assistant's default visibility is context-local. Enumerating, reading, focusing, closing, or injecting into a pane outside the origin context requires an explicit cross-context capability that the user grants visibly and can revoke.

Every pane-addressed action takes the origin context as a parameter and resolves the target within it. Cross-context reach is not a filter applied to a global result; the global lookup is not the default path. A grant issued in one context never satisfies a request from another, and workspace-global grants — a grant record with no workspace binding — are not available to the Assistant.

### Filesystem authority

Filesystem authority is a set of explicitly granted capability roots. The context root is the default root and the only one present without a decision. There is no implicit global root; installed-app directories are a normal grantable root like any other.

Containment is proven on the canonical path, with symlinks not followed, for existing paths and for not-yet-existing descendants alike. Lexical prefix comparison is never a boundary. The window between checking a path and using it is closed by carrying the resolved handle or path forward from the authorization, so a replacement race cannot swap the target underneath an approved operation. Directory walks do not traverse symlinks. Bounded size, line, match, and depth limits stay in force; they are resource protection, not authority.

### Process execution

Two distinct planes, permanently separated:

**The human terminal** is a PTY a person is watching. Injection into it is a human affordance. It stays context-scoped and bound to the origin pane's own linked terminal, it stays echoed so the human sees what was sent, and it is never a data channel between the Assistant and anything else.

**The command worker** is how the Assistant runs project work. It is a non-visible execution surface with a typed contract: an argv vector rather than a shell string, an explicit working directory drawn from an approved root, an explicitly constructed environment rather than an inherited one, structured streaming stdout and stderr, cancellation, timeouts, process-group isolation with process-tree cleanup, exit status, and background-job identity that survives the turn that started it.

Grants bind the exact command shape — program plus normalized arguments plus working directory. There is no persistent grant for arbitrary shell execution; a request whose program or arguments differ from the grant is a new decision. Commands in destructive classes always ask, regardless of posture and regardless of any existing grant. The existing narrow build path is subsumed by this worker rather than widened in place, and its residual escalation closes because the worker's sandbox applies to the interpreter it spawns.

The worker's policy binds the whole process tree. A child process, a shell startup file, an inherited descriptor, or a background job cannot exceed the filesystem and network policy of the worker that spawned it.

### Workspace secrets

A secret is an environment capability, never model context. A grant binds the canonical secret name, the destination command shape and working directory, the workspace, the actor, and a duration — five fields, all required.

Values are resolved at process-construction time and injected directly into the child's environment. They never enter a prompt, a transcript, a tool result, a command preview, an audit record, or an error message. The audit trail records that a named secret was injected into a named command; it records nothing about the value. A worker receives only the names explicitly injected for that invocation, not the host's ambient environment.

Because injected values live in a running process's environment, terminal-read and command-output paths are secret-adjacent surfaces and are not eligible for unprompted access. Redaction of known secret values on egress is a backstop for accidents, not the boundary.

### Connector trust

An app's declared schema, description, and mutability claim are model-facing hints. Host policy decides authority.

A connector tool's effective risk class is assigned by the host from the capabilities the app actually holds and the adapter the call reaches. An app cannot lower its own risk class. A tool whose class the host cannot establish is ask-gated conservatively. There is no unprompted-invocation path whose key is a value the app supplies.

Connector grants bind to the app's package identity together with the connector and tool id, and to the resource the call names. Two apps exposing the same tool name are different grants. Reinstalling under a different package identity does not inherit the previous grant.

### Model context

A model request carries only what the current context needs and what the user has authorized. Unrelated panes' metadata and contents are absent from the request — not truncated, not summarized, absent. Terminal scrollback and app state reach the model only through an intentional tool call whose authorization the user can see and revoke.

Project instructions — the root and nested `AGENTS.md` files, compatible `CLAUDE.md` imports, explicitly granted additional roots, and path-scoped rules — are model context and never authority. An instruction file cannot grant a capability, widen a root, pre-approve a command, or alter a grant. It is attacker-reachable text like any other repository content.

### Turn input

One ordered content-block envelope serves every entrypoint: the pane composer, Quick Note, Notes panes, app connectors, tests, and headless clients. The behavioral contract — provenance, bounded decoding, provider-native image blocks, persistence of metadata rather than payloads, and structured acceptance and rejection events — is specified in [`assistant-host-app.md`](assistant-host-app.md) and not repeated here.

The authority rule for that envelope: an attachment reference is a claim, never a capability. A path in note Markdown, a screenshot reference, or a client-supplied filename is resolved canonically and then authorized as an exact-file request through the reference monitor. Resolution is relative to the source note or the declaring pane. Nothing scans the surrounding directory for related files. A remote URL is never fetched implicitly; it requires an approved network action first. A rejected attachment fails visibly and never reaches the provider.

### Migration and revocation

Existing persisted grants were recorded under identity fields that this design no longer accepts as sufficient — workspace-and-tool-name breadth, no resource, no arguments. They cannot be mechanically widened into the new shape, because the user never approved the resource the new shape would have to invent.

Existing broad grants are therefore invalidated rather than translated. The next matching request asks again, in the new narrow form, with the UI showing what previously would have been covered. Session-scoped state does not survive the transition. Revocation removes the grant and terminates authority derived from it: an in-flight worker holding a revoked secret or root is cancelled rather than allowed to finish.

The permission surface shows the Assistant's actual holdings, enumerated per actor, resource, and duration. Nothing the Assistant holds is invisible to that surface — the built-in bypass is removed, and built-in status becomes a default-posture input rather than an enforcement shortcut.

## Runtime Boundary Decision

**Decision: keep the model and tool loop in-process behind a mandatory host reference monitor, and put an OS-sandboxed helper-process boundary around execution workers — the command worker first. Wasmtime is rejected for this role.**

The reasoning starts from where the authority actually lives. The model and tool loop holds none. It parses provider responses, decides which typed action to propose, and formats results. Every dangerous thing happens in an adapter behind the monitor. Sandboxing the loop constrains a component whose only power is to ask, at the cost of putting an IPC boundary through the middle of the Assistant's hottest path — streaming deltas, permission round-trips, and pane state.

That cost is not neutral for security. Marshalling the loop out of process means encoding every authorization request as a wire message, and the resulting serialization surface is a larger and more error-prone place for authority bugs than the monitor call it replaces. Defense in depth that adds attack surface at the boundary it is meant to protect is a bad trade.

The blast radius that genuinely needs bounding is execution: a project command, its interpreter, and its process tree, running attacker-influenced code from the repository. That is where an OS sandbox does real work, because the policy is legible — these roots, this network posture, this process group — and because the worker is already a separate process with a typed contract. On macOS this is a sandboxed child with an explicit profile; the same contract holds on other platforms with their native mechanism, which is why the boundary is specified as a process contract rather than as a macOS API.

Wasmtime is the wrong tool here specifically. It is excellent for third-party app code, whose needs are expressible as a small capability set. The Assistant's execution worker exists to run the user's real toolchain — compilers, package managers, test runners, git — as native processes with filesystem and network access. Putting that behind WASI means granting back essentially everything the sandbox would have withheld, producing the appearance of containment without the substance, at a very high implementation cost. Reusing the app runtime here would also blur a boundary the product depends on staying sharp: third-party apps are sandboxed; the first-party Assistant is *authorized*. Those are different mechanisms answering different questions.

Host UI and authorization ownership stay in Plexi in all cases. The monitor never moves out of the host, and a worker never renders a permission prompt or evaluates a grant. A worker can only execute what it was handed.

This decision is scoped to defense in depth for the model and tool loop. It does not reopen the third-party app sandbox, and it does not propose sandboxing the Plexi desktop host.

## Required Security Proof

Each property is a testable claim, not a checklist item. Implementation is not complete while any of them is unproven.

- A pane in one context cannot enumerate, read, focus, close, or inject into a pane in another context without an explicit cross-context grant.
- A grant issued for one context, pane, path, command, connector, or actor does not satisfy a request naming a different one.
- File read, write, and edit cannot escape a granted root through `..`, an absolute path, a symlink, a replacement race, or a not-yet-existing descendant.
- A grant for one command does not authorize the same program with different arguments, a different program, or a different working directory.
- A worker sees only the secret names explicitly injected for that invocation, and no secret value appears in any model request, transcript, tool result, command preview, or audit record.
- A worker's child processes, shell startup files, inherited environment, and background descendants cannot exceed the worker's filesystem and network policy.
- A connector's self-declared mutability cannot produce an unprompted call that host policy would otherwise ask about.
- A remote-provider request contains no metadata or content from panes outside the current context and outside the authorized attachment set.
- A pane client and a headless client submitting the same envelope produce identical authorized provider content blocks, and a denied or invalid attachment never reaches the provider.
- A note handoff reads only the chosen assets after canonical resolution; relative traversal, unrelated assets in the same collection, oversized files, unsupported media, and unapproved remote URLs are all rejected visibly.
- Every deny, ask, allow, use, and revoke is attributable to the exact actor, resource, and arguments, and risk-bearing actions record outcome as well as intent.
- The permission surface enumerates every authority the Assistant holds, and revoking one terminates work already running under it.

## Implementation Decomposition

Implementation is split so that each piece is independently testable and lands behind the reference monitor rather than beside it. The ordering below is a dependency order, not a schedule, and the stint tasks that carry it are created separately.

**The reference monitor comes first and alone.** Widening `PermissionRequest` to carry resource identity, arguments, and origin; making grant matching compare every identity field; binding session approval to the same fields; adding argument fingerprinting; removing the built-in enforcement bypass; and invalidating the old broad grants. Nothing else in this list is safe to build before it, because everything else expresses its authority in its vocabulary. It is provable entirely through host tests on grant matching and evaluation, with no new capability attached.

**Context isolation of the pane and terminal surface** follows immediately, since it is the largest live gap and depends only on the monitor. Origin-context parameters on every pane-addressed action, the explicit cross-context capability, and removal of the unconditional global apps root. Proof is harness tests asserting that a second context's panes are invisible and unreachable.

**File tool completion** brings the read, write, edit, grep, and list surface fully under canonical, no-follow, race-resistant resolution against granted roots, with roots as first-class grantable resources.

**The command worker** is the largest single piece: argv contract, working-directory roots, constructed environment, streaming output, cancellation, timeouts, process-group cleanup, background job identity, exact-command grants, and the destructive-class always-ask policy. It subsumes the existing narrow build path rather than widening it.

**The sandbox profile for the worker** is deliberately a separate follow-on. The worker's typed contract is valuable and testable on its own, and the OS sandbox profile is a platform-specific layer applied to an already-correct process boundary. Splitting them keeps a platform detail from blocking the authority work.

**Workspace-secret environment injection** depends on the worker, since a secret's destination is a command. Five-field grants, injection at process construction, and egress redaction as a backstop.

**Project-instruction loading** is independent of the worker and can land in parallel: root and nested `AGENTS.md`, compatible `CLAUDE.md` imports, explicit additional roots, path-scoped rules, and the invariant that none of it can widen authority.

**Connector trust policy** is likewise independent: host-assigned risk classes, package-identity-bound grants, and removal of the self-asserted unprompted path.

**Edit checkpoints and background jobs** build on the file surface and the worker respectively, and each is testable against its own substrate.

**The shared turn-input envelope** unifies the pane and headless entrypoints, after which **Quick Note and Notes image handoff** and **provider multimodal dispatch** land on top of it as separate, separately-provable pieces.

**The installed-host general-project parity gate** is last and is a gate, not a feature: on a real installed build, in a non-Plexi repository, inspect, edit, build, test, use a workspace-scoped secret without disclosing it, recover from one failed command, and produce an auditable final diff.

## Decisions Needing Sign-Off

Everything above is decided. These three are decided provisionally and are worth an explicit human confirmation, because each trades user friction or migration pain for containment and the right balance is a product judgment.

**Invalidating existing broad grants rather than translating them.** This is the correct security answer — a grant the user gave under "this tool, this workspace" was never consent to a specific resource — but it means every existing Assistant user re-approves their common actions once. The alternative, a one-time migration prompt that lets a user re-issue the old grants in narrow form, is friendlier and weaker.

**Destructive-command classes always ask, with no grant able to suppress the prompt.** This is a deliberate hole in the grant model: it makes some approvals unrepeatable by design. A user running a destructive command in a loop will feel it. The judgment call is whether the classifier's scope is drawn tightly enough that this is rare.

**Terminal reads lose unprompted access.** Making terminal scrollback ask-gated is correct because injected secrets live in that environment, but it changes the feel of the Assistant noticeably — reading a terminal is currently free and frequent. A narrower alternative is unprompted reads only for terminals in the origin context that hold no injected secrets, which preserves most of the convenience and is meaningfully harder to reason about.
