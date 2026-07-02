# Marketplace Monetization: Accounts, Payments, and the No-License Model

Status: active.
Stint: 0338–0341 (execution), 0322 (re-scoped), 0323/0325 (downstream).
Parent: [`app-framework-marketplace.md`](app-framework-marketplace.md). Supersedes §4 of [`marketplace-hosted.md`](marketplace-hosted.md) (the 0021 paid-apps section) and fills the named billing placeholders in its §5.4. Everything else in `marketplace-hosted.md` stands.

This is the commercial-launch design doc: how paid apps are sold, how accounts work, how money moves, and why there is no client-side licensing. It resolves every open decision in stint 0315.

## Decisions at a Glance

| Decision | Value |
|---|---|
| Client-side licensing | **None** — rejected (see below) |
| Proof of purchase | A row in the plexiapp.com database, keyed on account |
| Payment processor | Polar, as merchant of record — Plexi never stores payment data |
| Identity | Passwordless magic-link email auth, self-hosted on plexiapp.com |
| CLI login | Device-code flow: CLI requests, user clicks emailed link, CLI polls for bearer token |
| Revenue share | 85/15 publisher/Plexi, on net after Polar fees |
| Refund window | 14 days, no questions asked; refund deletes the purchase row, installed code untouched |
| AI subscription | 50 free requests/month, then $10/month Pro; overage hard-stops (`subscription_quota_exceeded`), never a surprise bill |
| Payment-required wire format | Extensible 402 envelope (see below) |
| Wallet / micro-transactions | Penciled in as closed-loop **credits**, server-side only, no work now |

## The Model: The Paid Download Is the Product

Buy it, get the files, they are yours like a book. A purchase is a server-side fact: *this account owns this app.* The host never checks anything at run time or install time — it presents the account bearer token when downloading a paid artifact, and the server decides. Protection levers, strongest to weakest (amended from the 0315 decided model):

1. **Gated downloads** — paid artifacts (initial install *and* every update) download only for accounts with the purchase row. Pirated copies work forever but rot: they can read update metadata, they cannot pull the files. Primary lever.
2. **Convenience asymmetry** — one-click buy vs. finding/trusting a pirated copy. Pirated copies don't list, don't get the trust sheet, don't get support.
3. **WASM obfuscation tier** — paid apps may ship compiled WASM instead of Python source. Raises "copy a .py" to "reverse-engineer a module."
4. **Selective server-brokered value** — opt-in per app; never the default. Costs local-first purity.
5. **Curated review + trust labels + ToS/DMCA takedowns** against bulk redistributors.

Hard DRM remains rejected: installed code and user state live on disk; nothing the server does can ever delete or disable what a user already has. True anti-forking is impossible for plain-text local apps and we do not attempt it. We monetize the relationship and the update stream, not the bits.

## Client-Side Licensing: Rejected, and Why

A license file on disk can only ever prove to *someone* that *you* bought the app. Every possible asker, audited:

- **The host at run time** — never asks; founding constraint.
- **The host at install time** — pointless; whoever can copy the app folder skips the install step entirely. The check only inconveniences the buyer.
- **The server at download time** — already holds the purchase record it issued; the license file is a second copy of a fact the server owns.
- **Against a reseller** — nothing technical survives `cp -r`. The real levers (delisting, update rot, DMCA) are all server-side and social.

The one thing a signed license enables — verifying a paid bundle fully offline — is a non-goal. Consequence for existing code: `License`, `LicenseStore`, and the `licenses/` directory in `src/app/marketplace.rs` are deleted; `PaymentProvider` is reshaped (purchase completes in the browser; the host observes the server, it never charges).

## Accounts

`AccountProvider` (`src/app/account.rs`) is already email-only — no password field exists, and it stays that way. No third-party IdP.

- **Web signup/login:** enter email → Resend magic link → clicked link establishes the session.
- **CLI login (`plexi account login`):** device-code flow. CLI asks plexiapp.com for a login code, the server emails a magic link, the user clicks it in any browser, the CLI polls and receives the opaque bearer token that `AccountSession` already stores at `<config_dir>/account.toml`. Tokens are stored hashed server-side.
- An account is required only to publish, buy, or use the AI subscription — the no-account rules in `marketplace-hosted.md` §1 are unchanged.

## Payments: Polar as Merchant of Record

Polar owns the checkout page, the card data, global sales tax/VAT, chargebacks, and publisher payouts. **Card entry always happens in the browser — no flow ever puts a payment form inside a pane.** Plexi's database stores accounts and entitlements only; payment instruments never touch our infrastructure.

Money path: buyer pays Polar → Polar webhook → plexiapp.com writes the purchase row → publisher receives 85% of net monthly via Polar payouts. Publishers see aggregate installs and revenue, never buyer identities.

## Purchase Flow

In-pane (`PLEXI_SOCKET` set — the normal case):

1. `plexi app install <app>` → registry returns the 402 envelope (checkout URL + purchase id).
2. CLI subscribes to the `marketplace::purchase` event stream, asks the host to spawn the marketplace app (sibling split) with `args = {app, intent: "purchase", purchase_id}`, and blocks with a timeout.
3. The marketplace pane opens on that app's detail view: trust sheet, price, buy button. Buy → browser → Polar checkout → webhook → purchase row.
4. The pane polls purchase state via `HttpFetch`; on purchase it asks the **host** to install (install logic lives in exactly one place, idempotent) and shows "installed — launch?".
5. `marketplace::install_completed {app_id, ok}` is emitted on the bus; the waiting CLI reports and exits 0. Timeout/abandon → nonzero exit, "purchase pending — re-run to resume." Re-running is safe: the server row is the state.

Headless (no `PLEXI_SOCKET` — SSH, CI, bare agent): the CLI prints the checkout URL, polls the server, and installs when the purchase lands. Same API, same exit semantics, no pane. Socket presence selects the mode.

Event-bus reporting is the primary path; server polling is the fallback. The handoff is built on the event bus only — never on the legacy pipe transports 0327 deletes.

## The 402 Envelope

Every payment-required response is a structured envelope, extensible by design:

```json
{ "reason": "purchase_required", "price": "12.00 USD",
  "options": [ { "type": "checkout", "url": "...", "purchase_id": "..." } ] }
```

Clients render options generically and ignore unknown types. A future server may add `{ "type": "credits", "balance": ..., "price": ... }` without any client change. This one rule is what keeps the credits future open at zero present cost.

## Refunds and Takedowns

- 14 days, no questions asked. Polar reverses the charge; the purchase row is deleted; the installed app keeps working but no longer re-downloads or updates.
- Delisting removes an app from the catalog and stops new sales. Existing installs keep working and — unlike refunds — existing purchasers keep their download/update access unless the takedown is for malware, in which case downloads stop and users are notified (never uninstalled).

## Plexi AI Subscription: The Numbers

Fills the named placeholders in `marketplace-hosted.md` §5.4 (mechanics live there; do not restate them):

- `FREE_TIER_REQUEST_COUNT` = **50 requests/month** per account.
- `PAID_TIER_PRICE` = **$10/month** (Polar subscription product).
- `PAID_TIER_REQUEST_ALLOWANCE` = generous fair-use ceiling, enforced server-side; the exact number is a proxy-config value, not a client constant.
- `OVERAGE_POLICY` = hard stop with `subscription_quota_exceeded`; upgrade or wait for the next cycle. Never silent rerouting, never a surprise bill. (Future: "or pay from credits.")

## Future: Credits (Penciled In, Not Built)

Closed-loop credits — bought via Polar, spendable only inside Plexi, non-withdrawable, non-transferable — for micro-transactions that don't warrant a checkout hop (per-use app features, AI overage). Never a "wallet" in the money-transmission sense. Requires only: a server-side balance table, Polar top-up products, and a `credits` option in the 402 envelope. No host changes, no stint until real demand exists.

## Infrastructure Summary

- **plexiapp.com (Astro/Node on Railway):** Postgres replaces the single-file SQLite; tables for accounts, auth tokens (hashed), purchases, subscriptions, publishers. API: device-flow auth, Polar webhooks, purchase-state reads, gated artifact downloads, the `ai.query` proxy endpoint.
- **Host:** `plexi account login/logout/status`, a real `AccountProvider`, bearer token on paid artifact downloads, 402 envelope handling, license machinery deleted.
- **Marketplace app:** a Core app on SDK v3 (`RegistryClient` catalog, trust sheet, purchase flow, event emission). The store itself is just an app.

## Done When

- A stranger can buy a paid app end-to-end (browser or in-pane) and re-install it on a second machine by logging in.
- No card data, license file, or client-side entitlement exists anywhere in the repo.
- A refunded purchase stops downloading but keeps running.
- The AI subscription free tier and Pro tier are purchasable and metered server-side.
