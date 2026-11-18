# api

The Voics REST API: accounts, users and settings, DMs, relationships, devices, guilds and their channels, roles, members, bans, invites, emojis, audit log, applications and bots, commands, interactions and webhooks. It also serves the internal P4 endpoints used by gateway and admin-bff.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the layering and component diagram.

Profile customization (api#43): `PATCH /users/@me` persists pronouns, a pair of RGB theme
colours, an accent colour and a banner. `PATCH /guilds/{guild}/members/{user}` accepts
self-only avatar, banner and bio overrides independently of nickname permissions.
Omitted fields stay unchanged; null clears an override. Images use the existing PNG,
JPEG, GIF and WebP data-URI upload rules with a 10 MiB decoded limit per image. Public
DTOs and transactional user/member events include the new fields. Server images use
`guilds/{guild}/users/{user}/{avatars|banners}/{hash}.{png|gif}`; clients fall back to the
global profile when overrides are cleared. Migration `0054` requires no new settings.
See [the profile specification](https://github.com/discordy-voics/docs/blob/feature/community-profiles/specs/2026-11-18-profile-customization.md).

## Run locally

Age-restricted background notifications (client#76): the internal channel push-targets
endpoint follows the channel's parent chain. Restricted or incomplete context forces
the existing token `previews` response field to false; session-local Continue never
enables background content previews. Accounts with persisted
`preferences.hide_age_restricted_channels: true` receive no restricted-channel push
target. Ordinary channels retain their registered preview preference. This policy
does not rewrite device tokens or require a notifier DTO change. Preference writes
are the separate API #151 companion. HTTP regressions live in
`crates/infrastructure/tests/http_push_tokens/age_restricted.rs`.

Prerequisites: Rust 1.99, Docker (for integration tests and local dependencies), and the C compiler,
Perl and Make required by vendored OpenSSL. On Windows use the MSVC toolchain with Strawberry
Perl (`OPENSSL_SRC_PERL` can point to a portable Perl executable). The Docker build installs its
own build prerequisites; the runtime does not need a system OpenSSL package.

```sh
cp .env.example .env
make dev
```

`make dev` expects Postgres on `5432`, Valkey on `6379`, Redpanda on `19092`, MinIO on `9000` and the messages service on `8082` (see the compose stack in `gitops/local`). The api listens on `8080`.

## Test

```sh
make test     # unit and integration tests; integration tests start containers
make lint     # rustfmt check and clippy with -D warnings
make build    # release binary
make docker   # OCI image, ghcr.io/discordy-voics/api:<version>
```

Integration tests start throwaway containers with testcontainers (Postgres 16, Redpanda v24.2.7, Valkey 8). Docker must be running.

## Community settings

`PATCH /guilds/{id}` accepts `features:["COMMUNITY"]` with `rules_channel_id`
and `public_updates_channel_id` (requires `MANAGE_GUILD`). Both must be
non-age-restricted Standard text/announcement channels in this guild; one channel
may serve both purposes. Configured channels cannot be deleted, made NSFW or
switched to ephemeral/P2P delivery until replaced or Community is disabled.
`features:[]` clears the pair and removes the discovery listing transactionally.
Ordinary guild updates preserve the pair; Community changes and channel writes
serialize on the guild row. Migration 0052 adds the channel pair and deferred FKs.

Discovery requires Community in addition to a description of at least 10
characters, the configured age/member thresholds and no NSFW channels. Existing
listing rows without Community become hidden at rollout. After explicitly
disabling Community, re-enabling does not automatically list the server again.
Search, join and report requests all check eligibility.

`POST /discovery/guilds/{id}/reports` accepts a report category and optional note
(500 characters). Only public listings can be reported by signed-in users;
owners cannot report their own listing. The existing 10/hour reporter limit and
duplicate-open-report behavior apply. Migration 0053 adds report kind `guild`.
The admin queue receives public name/description/category/tags, guild ID and the
owner at filing. No private channel/member/message data is copied. Resolving a
report does not automatically delist the server. See the central
[discovery specification](https://github.com/discordy-voics/docs/blob/develop/specs/2026-11-03-server-discovery.md).
No new configuration or infrastructure is needed. Membership screening remains
a separate feature; Community setup does not implement api#40.

Members with `MANAGE_GUILD` can use `GET|PATCH /guilds/{id}/vanity-url` to read,
set or remove a permanent invite. Send `{"code":"study-club"}` or
`{"code":null}`. Codes use 2–32 ASCII letters, digits or hyphens and resolve
case-insensitively through the ordinary invite preview/accept routes. They
cannot collide with another guild or an ordinary invite. A text channel visible
to @everyone is required. Bans and guild limits still apply. Renaming resets the
join count; existing members do not increase it. Changes are audited.

`PATCH /guilds/{id}` accepts `audit_log_enabled` (default true) and
`audit_log_retention_days` (30, 90 or 365; default 90). Only the owner or a member
with `ADMINISTRATOR` may change these. Disabled logging suppresses new audit
entries and their dispatches in the transaction; audit-policy changes always
remain recorded. Existing entries remain readable with `VIEW_AUDIT_LOG` until
retention expires. Queries enforce retention immediately, and an idempotent
worker deletes expired entries and their outbox copies at startup and hourly,
including in disabled guilds. The default 90-day retention also applies to
existing entries on rollout. Migration 0050 adds permanent invites; 0051 adds
audit policy. No new configuration or secret is needed.

## Server templates

`GET /guilds/templates/{code}` is a public, rate-limited preview of the saved
template. Creating from it still requires a signed-in user; managing a guild's
template requires `MANAGE_GUILD`. The five built-ins create their full roles,
channels and overwrites. `is_dirty` reports differences from the current
template-visible guild structure, while previews and creation keep using the
saved snapshot until explicit sync. Duplicate creation, including a database
race, returns error `30031`. No migration or new environment variable is needed.

## Configuration

All configuration comes from environment variables, validated at startup. Missing or invalid values stop the process with a message naming the variable.

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `DISCORDY_ENV` | no | `local` | `local` or `production` |
| `LOG_LEVEL` | no | `info` | Tracing filter |
| `API_BIND_ADDR` | no | `0.0.0.0:8080` | HTTP listen address |
| `DATABASE_URL` | yes | | Postgres (`discordy`) |
| `VALKEY_URL` | yes | | Rate limit buckets |
| `KAFKA_BROKERS` | yes | | Comma-separated Redpanda brokers |
| `INTERNAL_SERVICE_TOKEN` | yes | | Shared secret for `/internal/v1` (`X-Internal-Token`) |
| `JWT_PRIVATE_KEY` | yes | | Ed25519 PEM; `\n` escapes allowed |
| `JWT_PUBLIC_KEY` | yes | | Ed25519 PEM |
| `WEBAUTHN_RP_ID` | paired | | Passkey relying-party hostname, e.g. `auth.example.com`; set together with `WEBAUTHN_RP_ORIGIN` |
| `WEBAUTHN_RP_ORIGIN` | paired | | Exact HTTPS origin for ceremonies, including a non-default port if used. HTTP `localhost` is accepted only with `DISCORDY_ENV=local`. Unset both to disable new WebAuthn ceremonies |
| `SNOWFLAKE_WORKER_ID` | yes | | Preferred API worker 0–63; allocation uses 1–63 and permanently excludes legacy worker 0 |
| `PUBLIC_API_URL` | yes | | Base for webhook URLs |
| `PUBLIC_GATEWAY_URL` | yes | | Gateway URL advertised to clients |
| `CDN_PUBLIC_URL` | yes | | Public asset base |
| `MESSAGES_INTERNAL_URL` | yes | | messages service internal base |
| `MINIO_ENDPOINT` | yes | | S3 API endpoint |
| `MINIO_ACCESS_KEY` | yes | | |
| `MINIO_SECRET_KEY` | yes | | |
| `MINIO_BUCKET_ASSETS` | no | `discordy-assets` | Asset bucket |
| `DEFAULT_VOICE_REGION` | no | `nl-home` | Region for new guilds |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | no | | Enables OTLP/HTTP trace export |
| `DEVICE_MAX_IDLE_DAYS` | no | `30` | A device not seen for this many days (registered, or its session refreshed its tokens) is left out of E2EE device listings (`GET /users/{id}/devices`, `USER_DEVICES_UPDATE`, key-package claims) and deleted by the periodic prune together with its key packages |
| `DISCOVERY_MIN_MEMBERS` | no | `5` | Members a guild needs before it can opt in to public server discovery (and to stay listed) |
| `DISCOVERY_MIN_GUILD_AGE_DAYS` | no | `7` | Age in days a guild needs before it can be listed in discovery |
| `GROUP_DM_MAX_PARTICIPANTS` | no | `25` | Most members a group DM can have, the creator included (at least 2). Applies to creating a group and adding recipients; existing larger groups keep working. Peer-to-peer delivery stays limited to 10 members |
| `PASSWORD_HASH_CONCURRENCY` | no | `2` | Argon2id hashes/verifies (register, login, password change and reset, account delete, 2FA disable) that may run at once. Each needs one 19 MiB argon2 matrix (`m_cost` 19456 KiB, the crate default). The matrices are pooled and reused, so the memory need is exactly concurrency x ~19.5 MiB on top of the api baseline (about 11 MiB) and stays flat (api#119: allocating a fresh matrix per hash made glibc malloc retain over 300 MiB after 60 sequential registrations). Extra requests queue |
| `PASSWORD_HASH_QUEUE_TIMEOUT_MS` | no | `5000` | How long a request waits for a free hash slot. After that it gets `429` with a `Retry-After` header (the same shape as the rate limiter, which clients already retry), identical on every password endpoint so login leaks nothing beyond the status |
| `DEVICE_PRUNE_INTERVAL_SECS` | no | `3600` | Seconds between stale-device prune runs; it also drops push tokens not refreshed for `DEVICE_MAX_IDLE_DAYS` |
| `CORS_ALLOWED_ORIGINS` | no | `null,http://localhost:5173` | Comma-separated origins allowed by CORS (`null` is the packaged `file://` client) |

Push (mobile): `POST /api/v1/users/@me/devices/push` with `{platform: "apns"|"fcm", token, environment?: "production"|"sandbox", previews?: bool, only_mentions?: bool}` stores the calling session's token (call it on every start to refresh); `DELETE` on the same path removes it. VoIP (iPhone calls): `POST /api/v1/users/@me/devices/voip` with `{token, environment?: "production"|"sandbox"}` stores the calling session's PushKit token (call it every time PushKit reports it); `DELETE` on the same path removes it. It is separate from the notification token and works when notifications are off. Internal, for the notifier: `GET /internal/v1/channels/{id}/push-targets?author_id=` (each target also lists `voip_tokens`) and `DELETE /internal/v1/push-tokens` (removes the token from both tables). For manual pushes from the admin portal: `GET /internal/v1/users/{id}/push-tokens` and `GET /internal/v1/push-tokens?after=&limit=` (live notification tokens of one user, or a page of all users; `next_after` continues).

Token registration may finish before E2EE device registration. Binding a session to a device moves
its existing notification token too. Removing, evicting or pruning that device atomically removes
both notification and PushKit tokens for its bound sessions, unbinds those sessions and deletes the
device with its update event; other devices keep their tokens. The session remains signed in and
may register a fresh token later. Tokens never registered against a device still expire through the
token-age prune or session logout. A delayed registration rechecks and locks the live session inside
its database transaction, so it cannot restore tokens after logout. Device binding/removal and token
writes serialize per user through a transaction-scoped advisory lock, acquired before session locks;
the token write uses the session's current device rather than an earlier request snapshot.

`cargo test -p infrastructure --test http_push_lifecycle` covers registration order, device pruning,
device replacement, delayed/concurrent logout and cleanup rollback with disposable PostgreSQL.
These tests do not claim real APNs/FCM/PushKit delivery on a device.

Mention-policy directory: `GET /internal/v1/channels/{id}/notification-recipients?author_id=...`
requires `X-Internal-Token` and returns current permitted viewers independently of mobile tokens.
Each recipient has `user_id`, `role_ids`, `online`, resolved `level` (0/1/2), `muted`,
`suppress_everyone` and `suppress_roles`; device/token data is absent. The response also carries
`channel_id`, `guild_id`, `delivery_mode`, `evaluated_at` and `next_after`.
`limit` is 1–200 (default100) candidates examined, ordered by user ID. Continue with
`after=next_after` until null, even after an empty page caused by author/permission filtering.
Guild membership queries read at most `limit+1` profiles. Threads inherit parent access;
private-thread access follows the existing creator/manager rules. Ephemeral/P2P delivery returns
an empty terminal page. Each page resolves current policy and mute expiry independently; it is
not a historical or cross-page snapshot. Lookup failure returns an error, never an unfiltered
audience. Push delivery and this endpoint share the same preference resolver.

This endpoint is a prerequisite for messages#23. It does not itself change stored mention counts,
ACK behavior, silent-message semantics or counter idempotence. Those remain messages-service work.
`cargo test -p infrastructure --test http_notification_recipients` exercises the actual route with
disposable PostgreSQL, including users without tokens, pagination, permissions, mutes and privacy.

Endpoints: `GET /healthz`, `GET /readyz`, `GET /metrics`. Metrics: `http_requests_total` and `http_request_duration_seconds` (method, route, status class), `db_query_duration_seconds`, and `rate_limit_decisions_total` (scope, result; Valkey token bucket).

API and messages reserve non-overlapping timestamp intervals before generating IDs. Readiness
includes a valid ID reservation. See [ID allocation and recovery](docs/id-reservations.md) for
replica capacity, legacy rollout preflight, failure behavior and the mandatory restore barrier.

## Pending owner actions

- The reusable workflows `discordy-voics/.github/.github/workflows/rust.yml` and `docker.yml` must exist on `main` before `ci.yml` can run.
- The messages service must expose `POST|PATCH|DELETE /internal/v1/channels/{id}/messages[/{mid}]`. P4 does not pin these routes, so api calls them for bot, webhook and interaction messages. Until they exist, those calls return 503.
- Interaction creation and callbacks are routed but have no HTTP contract test yet.
- Redpanda topics listed in `contracts::envelope::API_TOPICS` are created by the compose init container and the cluster, not by api.
- The `.env.example` JWT keys are dev-only and public; production keys must be generated per environment.

## Session revocation guarantees

`POST /auth/refresh` accepts an optional `request_id` UUID for crash recovery.
Clients must persist a random UUID with the current refresh token before sending,
reuse that pair after response loss, and remove the UUID only after saving the
successor credentials. For five minutes after a committed rotation, identical
requests recover its immediate unused, unexpired successor in a live session.
The original token must also remain unexpired. Retries neither create token
branches nor extend expiry or last-used time. Legacy requests without a UUID,
mismatched IDs, expired windows and used successors keep strict reuse detection.
Logout, remote revocation and password changes cannot be undone by a retry.

Migration 0080 stores only the request UUID hash and successor token hash. The
successor is derived with HMAC-SHA256 from the previous bearer token, a fixed
purpose label and the UUID; plaintext tokens and request IDs are never stored or
logged. No new server secret or configuration is required. Roll out the API and
migration before clients depend on recovery; older clients remain compatible.

Access tokens continue to use Ed25519/EdDSA with `jsonwebtoken` 12. Its RustCrypto dependency chain
uses `elliptic-curve` 0.14 and `crypto-bigint` 0.7, so the former RUSTSEC-2026-0387 exception is removed.

Logout, remote logout, logout-all, suspension and refresh-token reuse persist the revoked session,
remove notification/VoIP tokens, and queue `SESSION_REVOKE` in the same Postgres transaction. An
outbox failure rolls the revocation back so retrying can still notify connected gateways. Device
cleanup follows the commit; a surviving session keeps its shared E2EE device.

`cargo test -p infrastructure --test http_sessions` covers a failed outbox write, replay of a rotated
token, and concurrent refresh versus logout-all, using isolated Postgres containers. The historical
one-off failure in api#121 did not reproduce in 12 repetitions with CPU pressure; it is not assumed
to have the same cause as these separately reproduced security defects.

## Passkey second factors

Passkeys complement password login and TOTP. `POST /users/@me/mfa/passkeys/register/start`
requires the current password and a name. The matching `/register/finish` verifies the signed
response and enables MFA only after a credential is stored. `GET /users/@me/mfa/passkeys`
returns names and usage dates; `DELETE /users/@me/mfa/passkeys/{id}` requires the password
and a TOTP or unused recovery code. These paths have the `/api/v1` prefix.

Password login returns error `60008`, a ticket, and the available `methods`. Clients pass the
ticket to `/auth/mfa/webauthn/start`, run the WebAuthn assertion, then submit the ticket,
`challenge_id` and `credential` to `/auth/mfa/webauthn/finish`. Binary WebAuthn fields use
base64url JSON, as in the contracts schemas. `/auth/mfa/totp` also accepts recovery codes
for accounts whose only second factor is a passkey.

Challenges expire after five minutes, bind to the initiating session or password-issued ticket,
and are consumed by the first correctly bound finish attempt, including an invalid assertion.
Postgres enforces consumption across replicas, the ten-passkey account limit, and monotonic
signature counters (authenticators that always send zero are supported). Credentials and
counter/ticket updates are transactional. Recovery codes use Argon2id hashes and are shown
once on first activation; adding a passkey preserves an existing set. Removing one factor
keeps MFA and recovery active when another factor remains. Purging an account deletes keys
and pending ceremonies. Passkey changes queue `USER_UPDATE` in the same transaction.

The verifier uses patched `webauthn-rs` 0.5.5 with vendored OpenSSL 3.6.3. Version 0.6.2's
`crypto-glue` currently requests the removed `rsa/sha2` feature and cannot resolve alongside
the stable RSA dependency in `jsonwebtoken` 12. JWT security fixes remain intact.

The configured RP origin must serve the actual browser ceremony. CORS, request headers,
`file://`, opaque `null` origins and arbitrary desktop custom schemes never establish RP
trust. Native clients need a ceremony at that HTTPS origin or an OS bridge with an associated
domain; these API endpoints alone do not provide that native integration. Keep RP ID/origin
stable after enrollment, and retain recovery codes before changing or disabling configuration.

`cargo test -p infrastructure --test http_passkeys --test http_passkey_recovery --test http_passkey_races --test postgres_passkeys`
uses disposable PostgreSQL containers and a software authenticator for registration/assertion,
origin/signature rejection, expiry/replay, concurrent replicas, counters, recovery and TOTP
coexistence. It requires no production credentials or external authenticator service.

`PATCH /users/@me/mfa/passkeys/{id}` accepts `{ "name": "Work laptop" }` for an owned key.
It trims and validates the 1–64-character label and changes no credential material,
signature counter, recovery code or MFA state. Unknown and foreign key IDs return the
same error. Run `cargo test -p infrastructure --test http_passkey_management` for the
HTTP/PostgreSQL ownership and recovery regression.

The optional `http_passkey_browser` test invokes the paired client/website acceptance
suite with real Chromium WebAuthn and an isolated PostgreSQL/API router. See the client
repository's `docs/passkeys.md` for the required environment and local TLS fixture;
it is intentionally ignored in ordinary API-only test runs. Test credentials travel
only through the child process environment, never command-line arguments or URLs.

## Integrations and bot consent

`GET /oauth2/tokens` lists only the authenticated user's explicit bot consent records;
`DELETE /oauth2/tokens/{id}` removes only that record. Personal revocation does not
uninstall a shared server bot or change its credentials. The supported authorization
scopes are `bot` and optional `applications.commands`; authorization-code flows and
personal-data OAuth scopes are not implemented. Unknown scopes and permission bits
are rejected. `POST /oauth2/authorize` returns `200` with a relative in-app location.
Each guild in the GET preview includes `installation_revision`; POST must echo the
selected guild's exact value. It distinguishes present, never-installed and removed
states. Stale revisions return 409 without writes, including removal before POST
starts and replacement installations with identical permissions. Missing/null
revisions return 400: the unreleased flow requires matched API/client/contracts
updates and intentionally has no legacy fallback. The existing 0070 ledger is reused.

Server managers list installed application bots with `GET /guilds/{id}/integrations`.
Removing one additionally requires Kick Members and role hierarchy. Removal clears
membership, managed roles and that installation's consent records atomically.
Migration `0070_application_authorizations.sql` tracks installation generations,
including a retained revision after removal, so delayed requests cannot undo a later
administrator removal. Existing installations receive no invented personal consent.

Webhook metadata edits write only supplied metadata. Authenticated
`POST /webhooks/{id}/reset-token` rotates the execution credential without changing
metadata; the old credential stops working after commit. Only create/reset responses
return a raw token and URL, with `Cache-Control: no-store`. Audit entries never contain
credentials. Lists and ordinary edits cannot recover an existing execution URL.

The interface is specified in contracts#49 and docs#178. Run
`cargo test -p infrastructure --test http_integrations --test http_applications --test http_command_index --test http_interactions`
for real PostgreSQL HTTP coverage, including forced stale-edit, concurrent-install,
remove/reinstall, removal-after-absent-read and transaction-rollback interleavings.

## Git hooks

Tests run locally, not in CI. Install [lefthook](https://github.com/evilmartians/lefthook) once and run `make hooks` (or `lefthook install`) to enable the pre-push hook in `lefthook.yml`: it runs this repository's lint and unit tests before every push. CI repeats only the slim gate on pull requests. Skip once with `LEFTHOOK=0 git push`.
