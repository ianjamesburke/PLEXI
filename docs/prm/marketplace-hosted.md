# Hosted Marketplace Spec (Sprint S4)

Status: spec for stint tasks `0018`-`0022`.
Parent: [`app-framework-marketplace.md`](app-framework-marketplace.md) — that PRM resolves conflicts with this doc.
Last updated: 2026-06-11.

This document specs the hosted marketplace lane: registry, install-from-registry, publisher review, paid apps, and the Plexi AI subscription. One rule governs all five: hosted services distribute metadata and broker money — they never become a dependency for running local apps.

## Stint Tasks

| Task | Title | Blocked by |
|---|---|---|
| [`0018`](../../.stint/tasks/0018-marketplace-hosted-registry-and-cdn.md) | Hosted registry and CDN | `0017` |
| [`0019`](../../.stint/tasks/0019-marketplace-publisher-submission-and-review-flow.md) | Publisher submission and review flow | `0018`, `0015` |
| [`0020`](../../.stint/tasks/0020-marketplace-browse-and-install-from-registry.md) | Browse and install from registry | `0019`, `0016` |
| [`0021`](../../.stint/tasks/0021-marketplace-paid-apps-license-and-revenue-model.md) | Paid apps, licenses, revenue model (spec only) | `0020` |
| [`0022`](../../.stint/tasks/0022-marketplace-plexi-ai-subscription-via-ai-query.md) | Plexi AI subscription via `ai.query` (spec only) | `0020` |

`0018`-`0020` are build tasks. `0021` and `0022` produce specs, not running payment or billing systems.

## 1. Hosted Registry and CDN (`0018`)

The registry is metadata distribution, not a platform dependency.

What it serves:

- An index of reviewed apps: slug, name, version, publisher, trust label, declared capabilities.
- Per-app metadata in the local package metadata format from `0015`. The registry does not invent its own schema — it republishes what the local validator already produces and checks.
- Package artifacts via CDN, addressed by checksum.

What it never requires:

- No hosted login for any local operation. Installing a local package, running installed apps, and browsing free apps all work without an account.
- Login is required only to publish, to buy a paid app, or to use the Plexi AI subscription.
- The registry being down never breaks an installed app. Installed code and user state live on disk; the registry is a discovery and download surface, nothing more.

## 2. Browse and Install from Registry (`0020`)

This is the concrete marketplace-up moment: a user discovers a reviewed app, inspects it, installs it, and runs it.

- Remote install reuses the local package trust sheet from `0016` — the same manifest display, runtime trust label, and capability list a local install shows. Remote install never bypasses it.
- Free hosted apps install without an account.
- The installed app records its source and version so the host can check the registry for update metadata later. Update checks are read-only metadata fetches, subject to the same no-login rule.
- After install, the app is a normal local app. Nothing about it phones home to the registry.

## 3. Publisher Submission and Review (`0019`)

One validator, two call sites:

- The publisher runs `plexi app validate` locally before submitting. Submission runs the same validator with the same rules server-side. There is no parallel hosted validator — if the rules diverge, that is a bug in `0019`.
- Automated checks are the `0015` validator plus the bypass-pattern scan (subprocess use, socket use, path traversal) defined in the parent PRM.

Human review is required for every reviewed-native Python app. Python apps are native processes, not sandboxed — review is the trust mechanism, so it cannot be skipped or fully automated. The resulting trust label is `Reviewed native process`, and the label must never claim isolation.

Submission states: submitted → in review → approved and listed, changes requested, or rejected. Listed apps can be delisted (see takedowns in `0021`).

## 4. Paid Apps, Licenses, and Revenue (`0021` — spec only)

This task writes the business spec before paid submissions open. It does not build payment enforcement. Free hosted install (`0020`) ships without any of it, and paid-app enforcement does not block v1 unless "marketplace v1" explicitly means commercial launch.

The spec must cover:

- Purchase flow and what a license is: the metadata that proves ownership, where it lives, and how the host checks it.
- Refund window and process.
- Takedowns: who can delist an app, what happens to existing installs (they keep working — installed code stays on disk).
- Revenue share between publisher and Plexi.
- Publisher analytics boundaries: publishers see aggregate install and revenue numbers, never user data.

Hard constraint carried from the parent PRM: licensing is hosted, but installed code and user state remain on disk. A license check failure can block a new install or an update, not delete or disable what the user already has.

## 5. Plexi AI Subscription (`0022` — spec only)

The Plexi AI subscription is one backend choice for the `ai.query` capability, nothing more. It is a Plexi-managed LLM proxy that an account holder can select instead of bringing their own OpenRouter key or running local Ollama. This task writes the spec; it builds no billing code, no Rust changes, no metering server. The governing rule from the intro applies in full: the subscription brokers money — it never becomes a dependency for running local apps.

### 5.1 What exists today (the seams this spec slots into)

The `ai.query` path already exists end to end and routes between two backends with **zero account awareness**. Spec everything below against these real seams; do not invent parallel machinery.

- **The capability.** `Capability::AiQuery` (`src/app/permissions.rs`), wire string `"ai.query"`, description "Make AI calls through the Plexi broker." It is sensitive (`is_sensitive()` returns `true`), so the host shows a consent modal on first use. Its `config_missing_reason()` returns `Some("ai.query requires [ai] config")` when `config.ai` is `None` — meaning an app declaring `ai.query` already refuses to launch on a machine with no `[ai]` section at all. That gate is config-driven and **account-blind**, and it must stay that way.
- **Backend selection.** `LiveAiBroker::dispatch` (`src/plexi_ai/broker.rs`) reads `ai_config.backend.as_deref().unwrap_or("openrouter")` and matches: `"ollama"` → `dispatch_ollama`, anything else → `dispatch_openrouter`. This single `match` is the routing seam. It has no notion of an account, a session, or a subscription today.
- **Backend config.** `AiConfig` (`src/config/mod.rs`) holds `backend: Option<String>`, plus typed sub-sections `openrouter: Option<OpenRouterBackendConfig>` and `ollama: Option<OllamaBackendConfig>`. Each sub-section carries its own `model_low` / `model_medium` / `model_high` tier strings; OpenRouter also carries `api_key_env` (default `OPENROUTER_API_KEY`), Ollama carries `host`. Default config (`scripts/default-config.toml`) ships `backend = "openrouter"` with a populated `[ai.openrouter]` and a commented `[ai.ollama]`.
- **Billing model enum.** `BillingModel` (`src/plexi_ai/backend/mod.rs`) already distinguishes `Metered` (per-token USD, pre-flight enforcement against a dollar envelope) from `Subscription` (flat-rate upstream; rate limits enforced by the provider, not Plexi AI). OpenRouter dispatches as `Metered`; Ollama dispatches as `Subscription`. The managed backend is the first **paid** `Subscription` backend.
- **Spend caps.** `AiConfig::effective_per_app_daily_usd()` (default `$1.00`) and `AiConfig::effective_global_daily_usd()` (default `$10.00`) feed `ledger::check_budget(&request.app_id, ai_config)`, called at the top of every `dispatch` before any backend runs. The ledger (`src/plexi_ai/ledger.rs`) sums today's per-app and global USD from `LedgerRow`s and denies with `budget_exceeded: …` when over.
- **The account.** `AccountSession` (`src/app/account.rs`) is the on-disk proof of login at `<config_dir>/account.toml` — `account_id`, `email`, opaque `token` (a bearer the host presents on authenticated requests but never interprets), and `provider`. `AccountProvider` is the auth seam; `account_provider()` returns `StubAccountProvider` (fails closed, `is_configured()` → `false`) until a real backend keyed on `marketplace.account_backend` drops in. Per the module doc, an account is required only to publish, to buy a paid app, **or to use the Plexi AI subscription** — those are the only three flows.

### 5.2 Backend routing — where the account check goes

The managed backend is a **third arm** of the existing `match` in `LiveAiBroker::dispatch`, selected by a new `backend` value (recommended: `"plexi"`):

```
"ollama" => dispatch_ollama(...)      // local, BillingModel::Subscription, no account
"plexi"  => dispatch_plexi(...)       // managed proxy, BillingModel::Subscription, account required
_        => dispatch_openrouter(...)  // user key, BillingModel::Metered, no account
```

The account requirement is a **pre-flight check inside `dispatch_plexi` only** — it is not hoisted into the shared top of `dispatch`, because hoisting it would couple the account to the local and BYO-key paths that must never require one. The check is: read the current `AccountSession` via `AccountStore::open().current()`; if `None`, fail with a clear, tagged error (parallel to today's `api_key_missing` / `ai_config_missing` tags — e.g. `account_required: log in with \`plexi account login\` to use the Plexi AI backend`). The session's opaque `token` is then presented as the bearer on every proxy request, exactly as `AccountSession::token`'s doc already describes ("the host presents it on authenticated requests (purchase, publish, subscription)"). The proxy URL defaults to `plexiapp.com` in code, mirroring how `marketplace::DEFAULT_REGISTRY_URL` defaults.

Stated plainly: **the routing layer has no account awareness today, and only the managed arm gains it.** The `"ollama"` and `"openrouter"` arms stay byte-for-byte account-blind.

### 5.3 Account requirement (and its strict inverse)

- The managed (`"plexi"`) backend **requires a logged-in `AccountSession`**. No session → fail closed with `account_required`, never a silent fallback to another backend.
- The local (`"ollama"`) and user-key (`"openrouter"`) backends **never require an account** and never read `AccountStore`. A machine with `account.toml` absent runs both at full capability.
- The capability consent modal (`is_sensitive`) is unchanged and orthogonal: it gates whether the *app* may call `ai.query` at all, independent of which backend the *host* is configured to use.

### 5.4 Metering and allowance

The managed backend is `BillingModel::Subscription`, so per-call USD is not pre-flighted the way `Metered` OpenRouter is. Two distinct ceilings apply and must not be conflated:

- **Local safety caps (already enforced):** `effective_per_app_daily_usd` and `effective_global_daily_usd` via `ledger::check_budget`. These are the user's own runaway-spend guardrails and apply to every backend including the subscription. They are not the subscription's commercial allowance.
- **Subscription allowance (server-side, billing spec):** a free-tier request count before payment is required, and paid-tier pricing above it. These numbers live in the billing spec and on the proxy server, **never** in app-framework code, `AiConfig`, or this doc. The host's job is only to present the bearer `token` and surface the proxy's allowance/quota errors back to the app as an `AiBrokerResponse` error, in the same shape as `budget_exceeded`. When the server reports the allowance exhausted, the host returns a tagged error (e.g. `subscription_quota_exceeded: …`) — it does not silently reroute to OpenRouter or Ollama.

Named placeholders for the billing spec to fill (do not invent values here): `FREE_TIER_REQUEST_COUNT`, `PAID_TIER_PRICE`, `PAID_TIER_REQUEST_ALLOWANCE` (or unmetered), `OVERAGE_POLICY`.

### 5.5 Separation from app purchase

Subscribing to Plexi AI and buying a paid app are **unrelated transactions with separate billing boundaries.** Concretely:

- App purchase runs through `PaymentProvider::purchase` → a `License` persisted by `LicenseStore` at `<config_dir>/licenses/<app_id>.toml` (`src/app/marketplace.rs`). A `License` proves ownership of one app id/version; it is never checked to *run* the app.
- The AI subscription is a recurring entitlement on the **account**, not a per-app `License`. It does not produce a `License`, is not stored in `LicenseStore`, and `PaymentProvider`/`payment_backend` does not handle it. It needs its own billing surface (recommended config selector: `marketplace.subscription_backend`, parallel to the existing `payment_backend` and `account_backend`), and its own server-side entitlement check keyed on `AccountSession::account_id`.
- The two share exactly one thing: the `AccountSession`. Owning a paid app's `License` grants no AI allowance; an active AI subscription grants no app licenses.

### 5.6 No-prerequisite rule (governing constraint)

**An app that declares the `ai.query` capability MUST work on a machine with local Ollama and no Plexi account.** The subscription is never required, never the default, and never a hidden fallback. The default config ships `backend = "openrouter"`; a user who sets `backend = "ollama"` and never logs in gets a fully functional `ai.query` for every installed app. The managed backend is opt-in by config (`backend = "plexi"`) and opt-in by login, and selecting it is the *only* path that ever touches an account. If the subscription proxy is down, only the `"plexi"` backend is affected — Ollama and BYO-key keep working, identical to the registry-down rule for installed apps.

### 5.7 Config recommendation

**Recommendation: add a thin `[ai.subscription]` sub-section, parallel to `[ai.openrouter]` and `[ai.ollama]`, but keep the account bearer `token` as the sole credential.**

- The bearer `token` on `AccountSession` is sufficient for authentication — do not duplicate or re-store it under `[ai]`. Auth lives on the account, period.
- But routing still needs the same per-tier knobs the other two backends expose. The managed proxy maps `ModelTier::Low/Medium/High` to upstream models server-side, yet the host benefits from an optional `endpoint` override (private/staging proxy, mirroring `registry_url`) and optional tier hints. A first-class typed `SubscriptionBackendConfig` (fields: optional `endpoint`, optional `model_low`/`model_medium`/`model_high` overrides) is more consistent with `OpenRouterBackendConfig`/`OllamaBackendConfig` than smuggling those into `[marketplace]` or hardcoding them.
- This keeps the boundary clean: **identity in `[marketplace]` (`account_backend`) and on-disk `account.toml`; AI routing in `[ai.subscription]`; commercial entitlement on the server.** It also matches the existing pattern where `AiConfig::overlay` already merges each backend sub-section independently.

A bare account token with no `[ai.subscription]` section is the cheaper path but breaks the symmetry the other two backends establish and leaves no place for an endpoint override — rejected on the 100-year-consistency standard.

## Done When

- A reviewed free app can be discovered, inspected with the standard trust sheet, installed, and run from the registry — with no account.
- A publisher can validate locally, submit, and pass automated checks plus human review using one shared validator.
- Paid apps, licenses, refunds, takedowns, revenue share, and analytics boundaries exist as a written spec.
- The Plexi AI subscription exists as a written spec for an `ai.query` backend, with the no-prerequisite rule stated.
