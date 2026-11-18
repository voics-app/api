# ID allocation and recovery

Migration `0060_snowflake_reservations.sql` supplies a persistent PostgreSQL allocator shared
by API and messages. `SNOWFLAKE_WORKER_ID` is a preference: API accepts 0–63 and allocates 1–63;
messages accepts 64–127 and allocates 65–127. Legacy workers 0 and 64 are never allocated.
Replicas may have identical configuration. Each startup generates a fresh random owner UUID.
The allocator locks the worker row, samples database time after the lock, commits a timestamp
interval, and only then returns it. End timestamps survive ownership changes.

`POST /internal/v1/id-reservations` requires `X-Internal-Token` and accepts service `messages`
only. API reserves directly through PostgreSQL. Requests choose acquire or renew; callers
cannot choose duration. No capacity (63 owners per service), unavailable database, clock
rollback, restore barrier or expired ownership returns 503. API startup fails if it cannot
reserve its own interval. Start API before messages.

The shared `voics-snowflake` crate enforces an exclusive wall-time end and a conservative
monotonic deadline anchored before the request. The remaining committed window is at most
30,000 ms. Runtime renews every five seconds; failures never extend the local deadline.
Expired issuance returns 503 and `/readyz` becomes unhealthy. `/healthz` remains a liveness
check. Recovery after expiry requires a process restart with a fresh owner, after resolving
the database/clock failure. No automatic worker acquisition or static-ID fallback occurs.
Monitor readiness and the redacted renewal warnings; logs expose allocated worker numbers,
not owner identifiers or internal credentials.

## First rollout

Before allowing old and new binaries to overlap, verify **effective runtime worker IDs for
every legacy API/messages issuer** are confined to 0 and 64 respectively. ConfigMap defaults
alone do not prove the effective environment. If that cannot be established, stop and fence
all old issuers before starting the new binaries (Recreate/drain rollout). Never run a legacy
static issuer on workers 1–63 or 65–127. Apply migration 0060, start API, then messages, verify
readiness and distinct allocated workers, then scale. Gateway/voice UUIDs do not use this store.

## Backup and restore barrier

The safety invariant requires one authoritative database history. Database clones must never
allocate simultaneously into the same ID namespace. Never truncate/delete reservation rows
or manually reuse owners. A restore is an operator operation, not an application retry.

1. Stop and fence **all** API/messages issuers, including partitioned or suspended processes.
   Block allocation during the operation. Before replacing the live database, capture its
   durable high-water mark outside the backup being restored:

   ```sql
   SELECT GREATEST((SELECT COALESCE(MAX(end_ms),0) FROM snowflake_reservations),
                   (SELECT not_before_ms FROM snowflake_allocation_guard)) AS previous_end_ms;
   ```

2. Restore the backup and verify PostgreSQL UTC against a trustworthy clock. Apply the checked-in
   barrier script with the captured integer (the script intentionally has no default):

   ```sh
   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -v previous_end_ms="$CAPTURED_END_MS" \
     -f scripts/restore-id-barrier.sql
   ```

   The transaction locks the global guard, clears owners and raises every worker's end and
   the allocation floor to the maximum of the prior floor, live capture, restored ends and
   current database UTC plus the 30,000 ms maximum reservation window. Reapplying it cannot
   lower the floor. It prints `wait_until_utc`. The guard is checked under a shared lock by
   every allocation and renewal, so allocation cannot race the barrier transaction.
   Input must be an integer in 1,767,225,600,000–3,966,248,855,551 ms; zero, negative,
   out-of-range, missing or non-integer inputs abort. A missing guard row also aborts. For
   a truly unused allocator whose capture is zero, supply an explicit trustworthy timestamp
   bound in this range instead of asserting that restored history is sufficient.

3. Keep issuers stopped until verified database UTC reaches `not_before_ms`. Then start API
   followed by messages with fresh process owners and confirm readiness. Reservations retain
   the barrier in their worker high-water marks, preventing reuse of pre-restore timestamps.

If a live capture is unavailable, do not substitute an arbitrary fixed sleep. Establish a
conservative upper bound from the last trustworthy issuance time and the maximum committed
future window, including any clock advance and backup refresh. Use that bound as the mandatory
script input. If rollback makes this upper bound unknown, keep issuance stopped until the
actual watermark is recovered. The script cannot infer lost history from an older backup.

## Verification

```sh
cargo test -p infrastructure --test id_reservations -- --test-threads=1
```

This starts isolated PostgreSQL 16 containers with random mapped ports. It covers 16 concurrent
allocators sharing the same preferred worker, 16,000 unique same-time IDs, all 63 slots busy,
separate production issuer instances, excluded legacy slots, reassignment and stale renewals,
time sampled after a row-lock wait, rollback denial, actual restore-script execution, internal
authentication, readiness failure and no user/outbox commit when issuance fails. Shared-crate
tests cover 4,096 IDs/ms, replayed responses, network delay and resumed expired processes.

`crates/snowflake/UPSTREAM.json` pins the canonical contracts commit and four source digests.
CI runs `python3 scripts/check-snowflake.py`; local verification with access to the canonical
Git objects additionally uses `python scripts/check-snowflake.py --canonical ../contracts`.
To update, pass `--canonical PATH --import-revision FULL_SHA` in both API and messages, then
rerun their tests. Build inputs remain inside each repository; no build credentials are needed.
