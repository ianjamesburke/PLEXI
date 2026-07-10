# Polar webhook fixtures

These payloads are the wire bodies Polar sends to our webhook endpoint. Each one
is validated against Polar's **own generated inbound schema** shipped in
`@polar-sh/sdk` (`Webhook<Event>Payload$inboundSchema`) — the same schema
`validateEvent` runs in production. A fixture that did not match Polar's real
shape would fail to parse and the tests would fail. The shapes are therefore
Polar's, not hand-invented.

Provenance note (never-mock rule): these were built by conforming to the SDK's
generated schema, not recorded from a live Polar sandbox — this environment had
no Polar organization credentials. When sandbox access is available, re-record
`order.paid` / `order.refunded` / `subscription.created` from a real test
transaction and diff against these; the field set should match. The `metadata`
block (`purchase_id`, `app_id`, `account_id`, `publisher`) is what our checkout
writes and Polar echoes back verbatim; tests overwrite those ids per-case.
