# api

The Voics REST API: accounts, users and settings, DMs, relationships, devices, guilds and their channels, roles, members, bans, invites, emojis, audit log, applications and bots, commands, interactions and webhooks. It also serves the internal P4 endpoints used by gateway and admin-bff.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the layering and component diagram.

## Run locally

Prerequisites: Rust 1.99, Docker (for the integration tests and the local dependencies).

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
| `SNOWFLAKE_WORKER_ID` | yes | | 0 to 1023 |
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

Endpoints: `GET /healthz`, `GET /readyz`, `GET /metrics`. Metrics: `http_requests_total` and `http_request_duration_seconds` (method, route, status class), `db_query_duration_seconds`, and `rate_limit_decisions_total` (scope, result; Valkey token bucket).

## Pending owner actions

- The reusable workflows `discordy-voics/.github/.github/workflows/rust.yml` and `docker.yml` must exist on `main` before `ci.yml` can run.
- The messages service must expose `POST|PATCH|DELETE /internal/v1/channels/{id}/messages[/{mid}]`. P4 does not pin these routes, so api calls them for bot, webhook and interaction messages. Until they exist, those calls return 503.
- Interaction creation and callbacks are routed but have no HTTP contract test yet.
- Redpanda topics listed in `contracts::envelope::API_TOPICS` are created by the compose init container and the cluster, not by api.
- The `.env.example` JWT keys are dev-only and public; production keys must be generated per environment.

## Git hooks

Tests run locally, not in CI. Install [lefthook](https://github.com/evilmartians/lefthook) once and run `make hooks` (or `lefthook install`) to enable the pre-push hook in `lefthook.yml`: it runs this repository's lint and unit tests before every push. CI repeats only the slim gate on pull requests. Skip once with `LEFTHOOK=0 git push`.
