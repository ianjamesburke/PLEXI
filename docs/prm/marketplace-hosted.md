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

The subscription is one backend choice for `ai.query`, nothing more.

- Apps call `ai.query`. The host routes it to local Ollama, user-owned keys, or a Plexi-managed subscription backend. Apps cannot tell which, and must not need to.
- The subscription is never a prerequisite for local apps or third-party apps. An app that declares `ai.query` works on a machine with local Ollama and no Plexi account.
- It is separate from app purchase. Buying an app and subscribing to Plexi AI are unrelated transactions.
- A free request allowance before payment is fine, but the number lives in the billing spec, not in app framework code or this doc.

## Done When

- A reviewed free app can be discovered, inspected with the standard trust sheet, installed, and run from the registry — with no account.
- A publisher can validate locally, submit, and pass automated checks plus human review using one shared validator.
- Paid apps, licenses, refunds, takedowns, revenue share, and analytics boundaries exist as a written spec.
- The Plexi AI subscription exists as a written spec for an `ai.query` backend, with the no-prerequisite rule stated.
