# hoshi-relay

Relay server binary + library.

## Responsibility
- Accept network traffic from clients/peers.
- Forward protocol traffic and maintain relay-side soft state required by the architecture.
- Stay simple: no content inspection, no durable chat persistence.

## Interactions
- Receives protocol traffic produced by `hoshi-netlib` clients.
- Exchanges control/health information with `hoshi-control-plane`.
- Participates in relay-to-relay peer-location announcements/lookups (via relay IDs + registry).

## Current Status
- UDP relay loop is implemented (`Relay::run`) with:
  - client registration (`Register`)
  - message forwarding (`SendMessage`)
  - sender ack + recipient delivery
  - unknown recipient / malformed packet errors
- Library exposes `Relay::bind` for test/runtime-configurable bind addresses.
- Control-plane integration and relay indexing/discovery are still pending.

## Boundary Guardrails
- Relay behavior should remain forwarding-focused with bounded soft-state only.
- Relay code should not own client UX policy or durable chat persistence.

## Test Commands
- Unit/default: `cargo test -p hoshi-relay`
- UDP tests run by default and self-skip when socket bind permissions are unavailable.
