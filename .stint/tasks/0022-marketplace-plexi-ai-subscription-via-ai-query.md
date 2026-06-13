---
id: "0022"
title: "Marketplace: Plexi AI subscription via ai.query"
status: done
estimate: "12h"
actual: "4m"
started_at: "2026-06-13T08:46:45Z"
completed_at: "2026-06-13T08:50:21Z"
sprint: "s4"
blocked_by:
  - 20
gh_issue: []
area:
  - "host/ai"
  - "infra/server"
tags:
  - "marketplace"
  - "ai"
  - "subscription"
---




Write the Plexi AI subscription spec. This is a **spec-only** task (no code). The output is a written document that defines how a Plexi-managed LLM proxy backend slots into the existing `ai.query` routing as one backend choice alongside local Ollama and user-owned API keys.

## Why

Apps call `ai.query`; the host routes to local Ollama, user-owned keys, or a Plexi-managed subscription. The subscription backend needs a written spec before any billing code ships.

## Spec must cover

- **Backend routing:** how the host selects the Plexi-managed backend vs Ollama vs user keys (today: `[ai].backend` in config, see `src/config/mod.rs` `AiConfig`).
- **Account requirement:** subscription requires a logged-in `AccountSession` (see `src/app/account.rs`), but free/local AI backends never require one.
- **Metering and allowance:** free tier request count, paid tier pricing, per-app and global daily caps (existing caps: `AiConfig::effective_per_app_daily_usd`, `AiConfig::effective_global_daily_usd`).
- **Separation from app purchase:** subscribing to Plexi AI and buying a paid app are unrelated transactions. The `PaymentProvider` trait in `src/app/marketplace.rs` handles app purchases; the AI subscription needs its own billing boundary.
- **No-prerequisite rule:** an app declaring `ai.query` capability must work on a machine with local Ollama and no Plexi account. The subscription is never required.

## Gotchas

- The subscription must not be a prerequisite for local apps.
- Request allowance numbers belong in the billing spec, not in app framework code.
- Do not modify any Rust source files. Output is a spec document.

## Existing code to reference (read-only)

- `src/config/mod.rs` lines 508-549: `AiConfig`, `OpenRouterBackendConfig`, `OllamaBackendConfig`
- `src/app/permissions.rs`: `Capability::AiQuery` definition
- `src/broker/mod.rs`: permission broker routing for `ai.query`
- `src/app/marketplace.rs`: `PaymentProvider` trait, `LicenseStore` (app purchase, separate from AI)
- `src/app/account.rs`: `AccountSession`, `AccountProvider` trait
- `scripts/default-config.toml` lines 64-79: `[marketplace]` config block

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md` (section 5)

## Outcome

Expanded `docs/prm/marketplace-hosted.md` §5 in place (kept cohesive with the
other four hosted lanes rather than a parallel doc). The spec covers: backend
routing (the managed proxy is a third arm of `LiveAiBroker::dispatch` in
`src/plexi_ai/broker.rs`, selected by `backend = "plexi"`, with the account check
*inside* `dispatch_plexi` only so the ollama/openrouter arms stay account-blind);
account requirement + strict inverse; metering (local `effective_*_daily_usd`
safety caps vs. server-side commercial allowance — numbers deferred to a billing
spec); separation from app purchase (`PaymentProvider`/`LicenseStore` is app
licenses, the subscription is an account-level entitlement); and the governing
no-prerequisite rule. Config recommendation: a thin typed `[ai.subscription]`
sub-section (endpoint + tier overrides) with the `AccountSession` bearer token as
the sole credential. Corrected the task's stale ref: the broker is
`src/plexi_ai/broker.rs`, not `src/broker/mod.rs`.

Open questions for the future billing-implementation task (named placeholders in
the spec, NOT invented): `FREE_TIER_REQUEST_COUNT`, `PAID_TIER_PRICE`,
`PAID_TIER_REQUEST_ALLOWANCE`/unmetered, `OVERAGE_POLICY`, and confirming the
`marketplace.subscription_backend` selector name. Spec-only task — no code, no
implementation scheduled yet.

## Variance

Estimate 12h. Spec-only; the work was reading the `ai.query` broker + account +
marketplace seams and writing §5 grounded in real types. Delegated the draft to a
subagent and verified every cited seam (`LiveAiBroker::dispatch`, `BillingModel`,
the daily-cap accessors) against the code before committing.
