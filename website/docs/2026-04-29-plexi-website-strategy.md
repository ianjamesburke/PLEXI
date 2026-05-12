# Plexi Website Strategy

> Locked in 2026-04-29. This document captures the strategic decisions that shape the landing page, the donor model, and the content cadence. Update it when something material changes — don't let it drift.

---

## TL;DR

Plexi's website exists to do three things, in this order:

1. **Convert YouTube viewers into GitHub Sponsors** funding a $3K/mo goal.
2. **Convert curious devs into commission customers** ($500 per custom Plexi app, with a tentpole video built around each one).
3. **Signal acquisition-readiness** to Cursor / Zed / Warp / Raycast — a real product, real users, a real founder, a real audience.

Donations are the bridge. YouTube is the moat. Acquisition is the outcome.

---

## Strategic Lockdown

### Funding model: bridge to acquisition

Donations exist to keep the lights on for 12–24 months while audience and adoption build to the point where one of the natural acquirers (Cursor, Zed, Warp, Raycast) sees Plexi as cheaper to buy than to clone. This is the **C-strategy** — donations are not the destination, they're the runway.

The **comfortable-solo donation path (B-strategy)** was rejected: solo OSS devs pulling $10K+/mo from sponsors is a population of ~50 people globally, almost all of whom spent 5+ years building niche reputation first. Not a plan you can underwrite a life on.

The **acquisition outcome** is realistic for this product set. PGAP + capability broker + the agent-built-app loop is a strategic asset for any of the named acquirers — they all have the same problem (feature-complete tools competing on AI integration, no protocol moat). Acqui-hire range $1–5M, strategic acquisition range $10–50M.

### The moat: agent-dev-loop + YouTube channel

Two-part moat, public and private:

- **Public/technical moat:** the agent-dev-loop. An agent can write a Plexi app, spawn it in the test harness, receive a PNG, simulate keypresses, assert on the next frame — all without a running GUI. This is a real, demonstrable engineering artifact that matters more every quarter as agents get better. Cursor and Warp do not have this. It's the thing the technical buyer cares about.
- **Private/distribution moat:** the YouTube channel. Cursor's eng team can clone PGAP in a quarter. They cannot clone a 50K-subscriber audience that trusts the founder. The audience is the actual acquisition asset — every tech acquisition in this range is 70% acqui-hire, and the audience comes with the founder.

### The content engine: daily shorts + weekly tentpole

Format C from the grilling — the only format that survives day 30:

- **1 tentpole video per week (10–18 min, polished):** a custom-commissioned Plexi app, built on camera. The donor is the protagonist. The app is the artifact. This drives YouTube subs, drives the watch-time metric the algorithm rewards, and drives commission demand by demonstrating what $500 actually buys.
- **6 shorts per week (60–90s, raw, phone-quality):** "what I shipped today," posted as YouTube Shorts / TikTok / X video. These build the daily presence, recyclable as social posts, and tolerate bad days (a 90s "today sucked, here's the bug" is *good* indie-dev content).

A pure-daily long-form schedule was rejected: it breaks every solo dev who tries it by week four. Tentpole + shorts matches the realistic energy curve.

The flywheel:
1. Viewer sees tentpole video → donates / commissions
2. Building the commissioned app → next tentpole video
3. Each app accumulates in `examples/` → ecosystem grows for free
4. Acquirer sees a founder shipping, a protocol with apps, and a marketing channel they don't have

### The donor model: GitHub Sponsors + commission split

**Patreon was rejected.** Patreon's model is gated tiers + exclusive content + members-only Discord — none of which Ian wants to manage. Patreon also takes 8–12% to provide infrastructure that isn't being used, and culturally trains donors to expect perks that don't exist here.

**The right stack:**

- **GitHub Sponsors as primary.** 0% fees (GitHub eats them), one-click for any GitHub user, recurring or one-time, sponsors auto-display on the GitHub profile (instant social proof), tax 1099 handled by GitHub/Stripe.
- **Open Collective as secondary** for non-GitHub donors and corporate sponsors who need invoices. Public ledger (every dollar in, every dollar out) is great trust signal and great B-roll.
- **Commissions are a separate flow**, not a sponsor tier. Single price ($500 flat), simple form (Tally / Formspree / mailto), email triage, Stripe Payment Link to close. Different page section, different mental model, different accounting bucket.

GitHub Sponsors does *not* support per-sponsor messaging beyond a single shared welcome message and an optional 200-char note when sponsoring. This is fine — commissions flow through their own form, not through a sponsor tier note.

### The funding bar: hero element

The single most important UI component on the landing page. Above the fold, prominent, live:

```
$847 / $3,000 monthly  ████████░░░░░░░░░░░  28% funded
   ↑ funded by 23 sponsors                   [Sponsor →]
```

Does five jobs at once: states the mission, creates urgency through visible gap, provides social proof, anchors the donate CTA to a tangible outcome, and becomes recurring video B-roll ("we're at $1,200 now, up $80 this week, thank you").

When $3K is reached, don't remove the bar — add the next goal ("$5K = Plexi gets a designer one day a week"). The bar never empties.

The psychological pattern is Kickstarter / NPR pledge drive / Wikipedia fundraiser. People don't donate to a person — they donate to closing a visible gap.

### Tax reality

"Donation" is a friendly label. The IRS treats every dollar received in exchange for work as **self-employment income**, regardless of what it's called. This applies to GitHub Sponsors, Open Collective, and commissions equally.

Practical rules:

- Track every dollar. All three platforms 1099 above $600/yr.
- Set aside ~30% for federal + state + self-employment tax (the SE portion is 15.3% on top of income tax).
- Pay quarterly estimated taxes. Skipping triggers penalties even if paid in full at year-end.
- Deduct legitimate business expenses (Plexi infra, hardware, software, home-office portion of rent).
- Form a single-member LLC once income crosses ~$30K/yr. $100–800 to set up depending on state. Liability separation matters: if a "donated" custom app deletes a donor's files, an LLC keeps personal assets out of reach.
- Do **not** attempt 501(c)(3) status. Massive overhead, board, IRS filing, restrictions on commercial activity. Not worth it for this size of operation.
- **Write a one-page Terms** before taking a single dollar: donations non-refundable, scope best-effort, no warranty. Otherwise commissioned-app failures become small-claims exposure.

---

## Landing Page Architecture

Locked elements every variant must include:

1. **Hero video** — auto-playing muted loop of building a Plexi app on camera. NOT a screenshot. The product is the human-plus-software, not the software alone.
2. **Funding bar** — current monthly recurring vs. $3K goal, sponsor count, prominent Sponsor CTA linking to GitHub Sponsors.
3. **Commission CTA** — equal weight to Sponsor button (or close to it). Opens a form, not a sponsor flow. Single price, queue position visible.
4. **Recent builds gallery** — last 6 commissioned apps with donor name (with permission), the brief, the video, the source link. Simultaneously social proof, portfolio, and content archive.
5. **Sponsor wall** — auto-pulled from GitHub Sponsors API. Names + avatars. Recognition is the entire perk.
6. **Public roadmap** — what's next, what's funded, what's blocked on dollars. Donation-gated where reasonable ("WASM sandbox: $X / $5K — donate to bump").
7. **Plexi the product** — short, demo-driven section. "One window, everything installs into it." Screenshot of the multiplexer with multiple panes (terminal, app, agent). Link to docs.
8. **YouTube channel embed** — recent uploads grid. The channel IS the marketing engine; surface it.
9. **Newsletter signup** — "Get new videos in your inbox." Single input + button, prominent band above the footer. Stored in our own DB; later piped into Buttondown or similar when scale warrants.

What's deliberately *not* on the page:
- No "Pricing" page (commissions are flat $500, no tiers to explain)
- No long-form "About Ian" page (the hero video is the about page)
- No login / dashboard / app store (v3.0 doesn't have a marketplace yet)

---

## POC Direction

Three POCs explore the Devtool ↔ Build-in-Public axis (the user's stated taste preference):

- **POC-A: Devtool-leaning.** Linear/Vercel aesthetic, polished grid, code samples, screenshots dominate. Funding bar present but understated. Signals "real product, ready for acquisition." Closest to Linear.app.
- **POC-B: Build-in-Public-leaning.** Funding bar IS the hero. Live commit feed. "Shipped today" cards. Sponsor wall prominent. Video embed. Signals "real human shipping in real time." Closest to levels.io.
- **POC-C: The fusion.** Linear-grade typography and spacing, but the *content* is the live build feed, funding bar, and sponsor wall. Polished container, raw contents. Hypothesis: this wins because it gets indie-dev authenticity (converts donors) AND production-grade polish (signals acquisition-readiness).

All three share the same locked elements and the same copy. The variable is the visual identity Ian commits to wearing for the next 18 months.

---

## Roadmap (website itself)

**Phase 0 — POCs (this week).** Three static HTML files in this repo. Pick a direction.

**Phase 1 — MVP site (week 1–2 after pick).** SvelteKit (matches Plexi tooling preferences). Static-generated, hosted on Vercel or Cloudflare Pages. Funding bar pulls from GitHub Sponsors API at build time + every 15 min via cron. Commission form via Tally embed → Ian's inbox. Sponsor wall auto-pulled. Domain: `plexi.app` if available, else `getplexi.com`.

**Phase 2 — Live data (month 2).** Funding bar polls GitHub Sponsors GraphQL API client-side every minute (cached). "Recent builds" section pulls from a hand-maintained `builds.json`. YouTube channel grid pulls from YouTube Data API.

**Phase 3 — Commission queue automation (month 3+).** Public queue position, automated Stripe Payment Link generation, automated "your build starts in N days" emails. Only build this if commission volume actually warrants it.

**Phase 4 — Marketplace (when v3.1+ ships WASM).** Public app directory, install with one command, search/browse. Out of scope until WASM sandbox lands and community apps are safe to install by default.

---

## Open questions deferred

- **Domain.** `plexi.app` vs `getplexi.com` vs other. Check availability before committing.
- **Newsletter.** Accepted as a YouTube distribution channel. Primary use case: one email per tentpole video upload, occasional build update. Not a paid tier, not gated content, not a substitute for the YouTube channel — a notification layer on top of it.
- **Discord vs Slack vs nothing.** No community channel currently planned. Sponsor-tier perk if added later, but managing a community is the same content overhead as Patreon and is currently out of scope.
- **Press / media kit.** Not needed until first acquirer conversation. Skeleton page when needed; not now.
