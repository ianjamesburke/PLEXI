---
id: "0022"
title: "Marketplace: Plexi AI subscription via ai.query"
status: todo
estimate: "12h"
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
