# hoshi-control-plane

Control-plane service.

## Responsibility
- Handle bootstrap and coordination concerns (auth, relay registry, monitoring).
- Stay off the realtime media hot path during normal operation.
- Provide operational visibility and policy surfaces for relays/clients.

## Interactions
- Clients (`hoshi-clientlib`/`hoshi-netlib`) fetch auth tokens and relay registry data.
- Relays publish health/load/reporting data.
- Does not carry end-to-end media streams.

## Current Status
- Tokio binary scaffold exists and starts successfully.
- Auth, registry, and monitoring endpoints are planned but not implemented yet.
