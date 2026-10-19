# Changelog

## 0.1.0

First release: everything on develop up to this point, deployed to gen-serv.

## 0.1.0

Initial release of the api service.

- Accounts: register, login with TOTP, refresh-token rotation with reuse detection, logout, EdDSA access tokens.
- Users, settings, DMs, delivery-mode overrides, relationships and devices with single-use key packages.
- Guilds, channels, threads, roles, members, bans, invites, emojis and the audit log, each mutation written with a transactional outbox event.
- Outbox relay publishing P6 envelopes to Redpanda.
- Applications, bot accounts, global and guild commands, OAuth2 bot authorization, interactions and webhooks.
- Internal P4 endpoints and admin statistics, user search and suspension.
- Valkey token-bucket rate limiting with `X-RateLimit-*` headers.
- Liveness, readiness and Prometheus metrics; optional OTLP trace export.
