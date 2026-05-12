# plexi-webapp

The website for [Plexi](https://github.com/ianjamesburke/PLEXI).

## Layout

- `docs/` — strategy, architecture decisions, copy notes
- `pocs/` — current landing-page proof of concept
- (future) `src/` — the actual SvelteKit site

## Status

Phase 0 → Phase 1 transition. Direction is locked on `pocs/poc-m-clean-dot.html` — dark mode, mono-only typeface, deeper amethyst palette, CRT scan-line texture, the P-of-panes logo (outlined left rect + outlined top-right square + filled violet bottom-right square), single "Download the App" hero CTA, and a donation-funded mission section in place of paid commissions.

```
open pocs/poc-m-clean-dot.html
```

Strategy lock: see [`docs/2026-04-29-plexi-website-strategy.md`](docs/2026-04-29-plexi-website-strategy.md).

Next: rebuild this POC as a real SvelteKit site, deploy to Vercel or Cloudflare Pages, wire the funding bar to the GitHub Sponsors GraphQL API.
