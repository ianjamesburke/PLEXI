# Native Browser Surface

Status: active

Stint: 0480, 0481, 0482, 0483, 0484, 0485, 0486, 0487, 0488, 0489, 0490

## Destination

Plexi can open a browser as a normal App pane. The first implementation is macOS-first and uses a native WebKit child view through Wry. It follows the browser lane already reserved by `app-framework-marketplace.md`; it does not add a new pane variant or bundle Chromium.

The host owns the native browser surface, its position, visibility, focus, storage, permissions, capture, and lifecycle. The browser app owns navigation state and browser chrome. A WASM app may request browser operations through a typed host capability later, but WebKit does not run inside the WASM component.

## Native Surface Contract

The browser surface is attached to the host window as a child view and keyed by pane id. On every layout change, the host projects the pane's content rectangle into native-view coordinates. The surface follows splits, tabs, zoom, context changes, portals, window moves, scale-factor changes, and pane closure. Hidden or covered browser panes do not intercept input.

Host input ownership remains authoritative. Plexi shortcuts work while the page has native focus. Page text entry, selection, clipboard, key events, and IME use the platform browser path. Browser focus and host focus cannot both claim the keyboard.

Egui overlays must never appear behind a child webview. The surface manager hides or substitutes the native view while host chrome must cover it, then restores it without losing page state.

## Screenshot Contract

`plexi host screenshot`, including `--pane`, captures the browser pixels through the sanctioned host pipeline. OS screen capture is not an acceptable substitute. The capture coordinator obtains a WebKit snapshot and composites it at the pane rectangle with the correct host chrome and overlay order. One acceptable implementation is to substitute the snapshot into the egui frame for the capture pass while temporarily hiding the child view.

If the first spike cannot produce correct browser pixels through `plexi host screenshot`, this architecture fails and dependent work remains blocked.

## Browser Profiles

A Plexi browser profile is a named browser identity backed by its own persistent WebKit website data store and history. It owns cookies, cache, local storage, IndexedDB, service-worker data, and browser history. Private profiles use a non-persistent store.

A Plexi context stores a preferred browser profile id. New browser panes inherit the source browser's profile when split from one, otherwise the context preference, otherwise the global default. A pane keeps its profile when moved between contexts. Explicitly switching a pane's profile updates the destination context preference.

Profiles are reusable across contexts. Plexi does not create a permanent profile for every context automatically, and context exports never contain credentials or website data.

Chrome profiles remain external identities. Plexi may import supported data into a Plexi-owned profile or route an external open to a selected installed Chrome profile. A WebKit pane never mounts Chrome's live profile directory.

## Keyboard Experience

The browser has host-navigation and page-focus modes. Host-navigation mode owns the address/search field, back and forward, reload, find, link hints, tab actions, focus movement, and open-in-pane commands. Page-focus mode sends ordinary input to the web page. Escape returns to host-navigation mode unless an active page interaction requires one first-stage dismissal.

The shortcut resolver states which chords remain Plexi-owned, which reach the page, and how a user explicitly sends a conflicting chord. This behavior is identical for physical input and `plexi pane key`.

## Agent Interface

The public CLI can open and close browser panes, navigate, focus the page or omnibar, send text and keys, read semantic state, traverse history, reload, capture a DOM/accessibility snapshot, click a stable element reference, type into a referenced control, evaluate an explicitly requested script, and inspect console or network failures.

Automation operates on the visible pane and its real WebKit instance. It does not start a separate Playwright browser. Stable references expire when the document changes and fail loudly when reused.

Semantic pane state includes URL, title, loading and failure state, navigation availability, focus owner, profile identity, page generation, viewport, automation readiness, and a bounded summary of console or network failures. It never exposes cookies, credentials, form secrets, or full page text by default.

## Permissions and Trust

Network navigation, downloads, file uploads, clipboard access, popups, notifications, camera, microphone, location, external schemes, certificate exceptions, and developer automation have explicit host policy. A page cannot gain filesystem or host control through a JavaScript bridge. Downloads land only through a user-approved destination policy and are observable in pane state.

Automation commands use the caller's existing pane authority and are logged. Remote debugging ports are not exposed. Browser data stays inside the selected Plexi profile.

## Required Gate for Every Task

Every task in this PRM ships its own deterministic live scenario. Before the task is accepted, its agent opens a PR, runs `just pr-install <PR>` from the feature worktree, starts a runner-owned host for that PR channel, opens the browser through the public Plexi CLI, and drives the behavior through the same CLI, socket, focus, and native-view paths a user reaches.

The scenario uses a controlled loopback fixture rather than a public website. It asserts semantic pane state and any persisted files, captures the pane with `plexi host screenshot`, opens the PNG, and records what was visually checked. A passing headless test, direct WebKit call, OS automation script, or screenshot that omits the page is not completion evidence.

Each failure bundle contains the scenario, command trace, semantic pane state, browser event tail, host log, profile-safe persistence metadata, and screenshot. Secrets, cookies, entered passwords, and unrestricted page contents are redacted.

Rust tests and headless scenes still cover pure state, command routing, layout projection, policy, and fake-engine behavior. The installed-host gate is additional and mandatory.

## Non-Goals

This work does not ship Google Chrome, Chrome Sync, Chrome extensions, password import, a general JavaScript-to-host bridge, Linux or Windows production support, or a Chromium/CEF renderer. Cross-platform engines remain possible after the macOS contract is proven.

## Upstream Boundaries

Wry supplies the Rust child-webview wrapper. WebKit supplies persistent identified website data stores. CMUX is a product and architecture reference, but its GPL source is not copied unless Plexi's licensing permits it. Any imported implementation records its source revision and license beside the code.
