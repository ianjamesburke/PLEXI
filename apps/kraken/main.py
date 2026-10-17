#!/usr/bin/env python3
"""Kraken — SDK v3 HTTP-backed market price watcher."""

from __future__ import annotations

import json
from typing import Any

from plexi_sdk import log, state
from plexi_sdk.effects import HttpFetch, SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import HttpResponse, KeyEvent, TimerFired, UiAction
from plexi_sdk.ui import (
    AppBar,
    Badge,
    Banner,
    Card,
    Column,
    Component,
    Divider,
    FooterKeys,
    KeyValue,
    Row,
    SelectList,
    Spacer,
    Text,
)

API = "https://api.kraken.com/0/public/Ticker"
TIMER_ID = 1
POLL_MS = 30_000
DEFAULT_PAIRS = ["XBTUSD", "ETHUSD", "SOLUSD"]
PAIR_ALIASES = {"XXBTZUSD": "XBTUSD", "XETHZUSD": "ETHUSD"}

DEFAULT_STATE: dict[str, Any] = {
    "pairs": DEFAULT_PAIRS,
    "prices": {},
    "selected": 0,
    "loading": False,
    "error": "",
}


def init(size, args) -> list:
    data = _state()
    if args:
        data["pairs"] = [str(p).upper() for p in args]
    effects: list = [
        SetTitle("Kraken"),
        SetTimer(TIMER_ID, POLL_MS, repeat=True),
    ]
    if not data["prices"] or args:
        data["loading"] = True
        effects.extend(
            [
                SetStatus("Loading prices"),
                SetState(data),
                _fetch(data["pairs"]),
            ]
        )
    else:
        data["loading"] = False
        effects.extend([SetStatus(_status(data)), SetState(data)])
    log.info(f"kraken: SDK v3 initialized pairs={','.join(data['pairs'])}")
    return effects


def update(event) -> list:
    data = _state()
    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        data["loading"] = True
        return [SetState(data), SetStatus("Refreshing prices"), _fetch(data["pairs"])]
    if isinstance(event, HttpResponse):
        data.update(_handle_http(data, event))
        return [SetState(data), SetStatus(_status(data))]
    if isinstance(event, UiAction) and event.handler_id.startswith("pair:"):
        try:
            idx = int(event.handler_id.split(":", 1)[1])
        except ValueError:
            return []
        data["selected"] = _clamp(idx, len(data["pairs"]))
        log.info(f"kraken: selected pair index={data['selected']}")
        return [SetState(data), SetStatus(_status(data))]
    if not isinstance(event, KeyEvent) or not event.pressed:
        return []
    if event.key in ("down", "j", "ArrowDown"):
        data["selected"] = _clamp(data["selected"] + 1, len(data["pairs"]))
    elif event.key in ("up", "k", "ArrowUp"):
        data["selected"] = _clamp(data["selected"] - 1, len(data["pairs"]))
    elif event.key == "r":
        data["loading"] = True
        return [SetState(data), SetStatus("Refreshing prices"), _fetch(data["pairs"])]
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    selected_pair = data["pairs"][data["selected"]] if data["pairs"] else ""
    price = data["prices"].get(selected_pair, {})
    rows = [
        {
            "name": str(data["prices"].get(pair, {}).get("label") or pair),
            "description": _price_line(pair, data["prices"].get(pair, {})),
            "trailing": str(
                data["prices"].get(pair, {}).get("change")
                or data["prices"].get(pair, {}).get("last")
                or "-"
            ),
        }
        for pair in data["pairs"]
    ]
    label = str(price.get("label") or selected_pair or "Ticker")
    detail_section: Component = (
        Banner(data["error"], tone="danger", title="Error")
        if data["error"]
        else Card(
            [
                Row(
                    [
                        Text(f"{label} Details", size=14.0, bold=True),
                        Spacer(grow=True),
                        Badge(selected_pair or "NONE", color="accent"),
                    ],
                    align="center",
                ),
                Divider(),
                KeyValue(
                    [
                        ("Last", str(price.get("last", "-"))),
                        ("24h Change", str(price.get("change", "-"))),
                        (
                            "Bid / Ask",
                            f"{price.get('bid', '-')} / {price.get('ask', '-')}",
                        ),
                        ("High", str(price.get("high", "-"))),
                        ("Low", str(price.get("low", "-"))),
                        ("Volume", str(price.get("volume", "-"))),
                    ]
                ),
            ],
            padding=16.0,
            gap=8.0,
        )
    )
    subtitle = "market ticker · refreshing…" if data["loading"] else "market ticker"
    return Column(
        [
            AppBar("Kraken", subtitle),
            SelectList(rows, selected_idx=data["selected"], on_select="pair")
            if rows
            else Text("No pairs configured.", size=12.0),
            detail_section,
            Spacer(grow=True),
            FooterKeys([("j/k", "select"), ("r", "refresh"), ("timer", "auto")]),
        ],
        grow=True,
    )


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    prices: dict[str, dict[str, Any]] = {
        str(k).upper(): dict(v)
        for k, v in (data.get("prices") or {}).items()
        if isinstance(v, dict)
    }
    raw_pairs = data.get("pairs") or DEFAULT_PAIRS
    norm_pairs: list[str] = []
    for item in raw_pairs:
        if isinstance(item, dict):
            code = str(
                item.get("pair")
                or item.get("symbol")
                or item.get("name")
                or item.get("label")
                or ""
            ).upper()
            if not code:
                continue
            norm_pairs.append(code)
            merged = dict(prices.get(code) or {})
            for field in ("label", "last", "change", "bid", "ask", "high", "low", "volume"):
                if item.get(field) is not None:
                    merged[field] = str(item[field])
            prices[code] = merged
        else:
            code = str(item).upper()
            if code:
                norm_pairs.append(code)
    data["pairs"] = norm_pairs or list(DEFAULT_PAIRS)
    data["prices"] = prices
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["pairs"]))
    data["loading"] = bool(data.get("loading"))
    data["error"] = str(data.get("error") or "")
    return data


def _fetch(pairs: list[str]) -> HttpFetch:
    return HttpFetch(
        f"{API}?pair={','.join(pairs)}", headers={"Accept": "application/json"}
    )


def _handle_http(data: dict, event: HttpResponse) -> dict:
    if event.status < 200 or event.status >= 300:
        data["loading"] = False
        data["error"] = f"HTTP {event.status}: {_body_text(event)[:240]}"
        log.warn(f"kraken: request failed {event.status}")
        return data
    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        data["loading"] = False
        data["error"] = str(exc)
        return data
    errors = payload.get("error") or []
    if errors:
        data["loading"] = False
        data["error"] = ", ".join(str(item) for item in errors)
        return data
    data["prices"] = _parse_prices(data["pairs"], payload.get("result") or {})
    data["loading"] = False
    data["error"] = ""
    log.info(f"kraken: loaded {len(data['prices'])} prices")
    return data


def _parse_prices(pairs: list[str], result: dict) -> dict:
    prices = {}
    remaining = list(pairs)
    for key, ticker in result.items():
        pair = PAIR_ALIASES.get(key.upper(), key.upper())
        for requested in remaining:
            if requested in pair or pair.endswith(requested):
                pair = requested
                break
        prices[pair] = {
            "last": _first(ticker.get("c")),
            "bid": _first(ticker.get("b")),
            "ask": _first(ticker.get("a")),
            "high": _first(ticker.get("h")),
            "low": _first(ticker.get("l")),
            "volume": _first(ticker.get("v")),
        }
    return prices


def _first(value) -> str:
    if isinstance(value, list) and value:
        return str(value[0])
    return "-"


def _body_text(event: HttpResponse) -> str:
    if isinstance(event.body, bytes):
        return event.body.decode("utf-8", errors="replace")
    if isinstance(event.body, list):
        return bytes(event.body).decode("utf-8", errors="replace")
    return str(event.body)


def _price_line(pair: str, price: dict) -> str:
    high = price.get("high", "-")
    low = price.get("low", "-")
    if high != "-" or low != "-":
        return f"last {price.get('last', '-')} · H {high} · L {low}"
    return f"last {price.get('last', '-')}"


def _status(data: dict) -> str:
    if data["loading"]:
        return "Loading"
    if data["error"]:
        return "Error"
    return f"{len(data['prices'])} prices"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
