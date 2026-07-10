# Polar webhook fixtures

These payloads are the wire bodies Polar sends to our webhook endpoint. Each one
is validated against Polar's **own generated inbound schema** shipped in
`@polar-sh/sdk` (`Webhook<Event>Payload$inboundSchema`) — the same schema
`validateEvent` runs in production. A fixture that did not match Polar's real
shape would fail to parse and the tests would fail.

## Provenance (never-mock rule)

Recorded from a **live Polar sandbox** (org `plexi-sandbox`) during stint 0355,
not hand-authored:

- `order.paid.json` / `order.refunded.json` — a real one-time app checkout was
  completed with Polar's test card and the resulting order fetched via
  `GET /v1/orders/{id}`; the refunded variant is the same order after a real
  `POST /v1/refunds`. The webhook `data` field is exactly this order resource.
- `subscription.created.json` — a real recurring ($10/mo) checkout was completed
  and the subscription fetched via `GET /v1/subscriptions/{id}`.

Every structural field, type, and enum (`status`, `billing_reason`,
`platform_fee_amount`, `net_amount`, `product_price`, `items`, …) is the live
sandbox value. Only the **identity** fields were canonicalized to the test's
paid app: the `metadata` block (`purchase_id`, `app_id`, `account_id`,
`publisher`) — which is what our checkout writes and Polar echoes back verbatim,
and which tests overwrite per-case — plus the embedded product name and the
customer's email/name/avatar. `platform_fee_amount` is the real 110 on a
$12.00 order, so `net_cents` = 1200 − 110 = 1090.
