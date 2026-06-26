# Issue #51 — `create_user_with_invite` SQLite transaction discipline: design

- Date: 2026-06-26
- Issue: [#51](https://github.com/jaunder-org/jaunder/issues/51) — `storage(sqlite): create_user_with_invite read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`
- Status: approved (design); awaiting spec review

## Problem

`SqliteAtomicOps::create_user_with_invite` (`storage/src/sqlite/mod.rs:149`) runs a
**read-then-write deferred transaction**:

```
BEGIN              -- sqlx pool.begin(): deferred, takes no lock
SELECT … FROM invites WHERE code = ?     -- shared (read) lock; validates invite
INSERT INTO users … RETURNING user_id    -- upgrades shared → reserved (write) lock
UPDATE invites SET used_at, used_by …    -- consumes the invite with the new user_id
COMMIT
```

The `SELECT`→`INSERT` shared→reserved **upgrade** is the exact `SQLITE_BUSY`-on-upgrade
failure mode documented in **ADR-0021** (the same class as #18): under a concurrent
writer in WAL mode the upgrade is against a stale snapshot, returns `SQLITE_BUSY`, and
`busy_timeout` cannot rescue it (retrying does not refresh the snapshot).

This is **not** a single-statement collapse (ADR-0021's preferred remedy): the operation
spans two tables with a mutual dependency — `INSERT users` produces the `user_id` that
`UPDATE invites.used_by` records, and neither side may land if the other fails. SQLite
cannot express a two-table atomic write in one statement (its CTEs are read-only; no
writable-CTE `WITH ins AS (INSERT…) UPDATE…`). Write-first reorderings to avoid an explicit
transaction were considered and rejected — each leaks a failure-path anomaly the current
transaction cleanly avoids:

- *Claim invite first, then insert user:* an `INSERT` failure (`UsernameTaken`) burns the
  invite with no user created — the user cannot retry. Regression.
- *Insert user first, then claim invite:* an invalid invite leaves an orphan user that must
  be deleted on the error path, and concurrent same-invite registrations race to delete
  their losers. Ugly; churns the `user_id` sequence.

Per ADR-0021: *"When multiple interdependent statements are genuinely required, take the
write lock up front with `BEGIN IMMEDIATE` rather than a deferred `BEGIN`."* That is this fix.

## Decision: take the write lock up front, keep validate-before-hash

Reshape the SQLite implementation only (the Postgres impl is MVCC-safe and unchanged) to:

1. Acquire a raw connection and issue **`BEGIN IMMEDIATE`** (write lock taken immediately —
   no shared→reserved upgrade can fail), mirroring the established precedent in
   `storage/src/sqlite/backup.rs:23,70`. sqlx's `Transaction` cannot be used here because
   `pool.begin()` issues its own deferred `BEGIN`; we drive `BEGIN IMMEDIATE`/`COMMIT`/
   `ROLLBACK` manually on the connection.
2. Keep the existing statement order — **validate first, then hash, then write** —
   so a bogus/invalid invite is rejected by the cheap `SELECT` **before** any Argon2 work
   (see *DoS posture* below). The argon2 hash therefore runs *inside* the immediate
   transaction on the success path only.
3. Wrap the body in the `async {}.await` + `match` pattern from `backup.rs` so **every** exit
   path issues an explicit `COMMIT` or `ROLLBACK`. The implicit drop-rollback that sqlx's
   `Transaction` provided is gone with the raw connection, and a pooled connection must never
   be returned to the pool with an open transaction.

### Resulting shape (SQLite)

```
                                                  -- NB: hash is NOT moved before the tx;
                                                  --     it stays after validation (DoS posture)
conn = pool.acquire()
BEGIN IMMEDIATE                                   -- write lock up front
  SELECT used_at, expires_at FROM invites WHERE code = ?   -- validate (3 error paths)
    └─ bad invite → ROLLBACK, return InviteNotFound/AlreadyUsed/Expired  (no hash)
  hash_password(password)                          -- success path only
  INSERT INTO users … RETURNING user_id            -- unique violation → ROLLBACK, UsernameTaken
  UPDATE invites SET used_at, used_by WHERE code = ?
COMMIT                                             -- success → return user_id
```

### DoS posture (why validate-before-hash, not hash-before-transaction)

Registration is **invite-gated**. The expensive operation is the Argon2id hash (~tens of
ms by design). Because the cheap invite `SELECT` runs *first*, a request bearing a
bogus/nonexistent invite code is rejected as `InviteNotFound` **without hashing**. This
preserves the system's only capability-based DoS lever: under attack, *stop issuing
invites* and the hashing surface drops to zero. Hashing *before* validating would destroy
that lever — anyone could burn CPU with garbage codes regardless of invite issuance.

This is safe with respect to **ADR-0018** (timing-equalized authentication) because the
thing being validated is a **high-entropy secret**, not an enumerable identifier — see
ADR-0022 below.

The cost retained by Option A: a *successful* (rare, invite-gated) registration holds the
single SQLite write lock across the hash. Accepted on YAGNI grounds given registration
frequency; the alternative (pre-read outside the lock + TOCTOU re-validation inside it) was
considered and judged not worth the added code.

## New ADR-0022 — validate cheap before expensive when the gate is a high-entropy secret

This fix is the first deliberate application of a discipline worth codifying, and it
borders ADR-0018, so it gets an ADR to draw the boundary precisely.

- **ADR-0018** governs validating an **enumerable identifier** (username/email at login):
  do *equal* work (dummy-hash on the absent path) so response timing cannot reveal account
  existence.
- **ADR-0022** governs validating a **high-entropy secret** (invite code, session/reset
  token — 256-bit random via `auth::generate_token`): a cheap lookup *before* expensive
  work (Argon2) is **safe** — a timing oracle gives no traction against a 2²⁵⁶ space — and
  **preferred**, because it (a) bounds DoS amplification and (b) preserves the
  capability-issuance throttle as the control lever.

Applications of ADR-0022:
- `create_user_with_invite` (invite code = secret): already validate-first; #51 preserves it.
- `confirm_password_reset` (reset token = secret): currently hashes the new password
  *before* validating the token → violates ADR-0022; tracked as a **new follow-up issue**
  (not fixed here — it has no capability gate, so it is strictly more exposed and deserves
  its own cycle).

The boundary is documented **bidirectionally**: ADR-0018 is amended with a short
scope-boundary cross-reference pointing to ADR-0022, so a reader who lands on the
"always equalize timing" decision learns that validating a high-entropy secret is
deliberately handled the opposite way. ADR-0018's decision and durable invariant are left
**immutable** — only a clarifying cross-reference (and an `Amended:` line) are added, never
a rewrite of the original outcome.

## Deliverables

1. `storage/src/sqlite/mod.rs` — reshape `create_user_with_invite` per the shape above.
2. `docs/adr/0022-validate-before-expensive-work.md` — new ADR (house style: `# NNNN. Title`,
   `- Status/Date/Deciders`, `## Context`/`## Decision`/`## Consequences`).
3. `docs/adr/0018-constant-time-authentication.md` — amend with a scope-boundary
   cross-reference to ADR-0022 and an `Amended: 2026-06-26` line; the original decision and
   durable invariant are unchanged.
4. `docs/README.md` — add the ADR-0022 row to the ADR table.
5. Tests (see below).
6. File the `confirm_password_reset` follow-up issue (jaunder-issues), assigned to the
   **Robustness** project.

## Backend parity

Backend parity is a **design invariant** (ADR-0019): the SQLite and Postgres impls diverge
structurally but must return identical results for identical inputs. This change is
SQLite-only — the Postgres impl keeps its deferred `begin()` (MVCC-safe) and is not
touched, so parity is preserved by construction.

How `create_user_with_invite` is tested today: `invite_and_atomic_registration_work`
(`server/tests/storage/storage.rs:719`, `#[apply(backends)]`) exercises it on **both**
backends for the happy path + `InviteAlreadyUsed`. The detailed error/rollback paths
(`InviteExpired`, `InviteNotFound`, `UsernameTaken`) are covered only by SQLite-only
`#[tokio::test]`s (`storage.rs:1242-1405`) — a Postgres coverage gap that is **not** in
scope here (it predates #51 and spans many storage functions); it is tracked generically as
**#54**. Because #51 does not touch the Postgres impl, it introduces no new Postgres risk.

## Testing

- **Behavior unchanged:** the existing SQLite functional tests
  (`server/tests/storage/storage.rs:1242-1405`, five `create_user_with_invite_*` cases) and
  the in-file `create_user_with_invite_hash_failure_returns_internal_error` test continue to
  pass; the four invite/username error paths (`InviteNotFound`, `InviteAlreadyUsed`,
  `InviteExpired`, `UsernameTaken`) each still return their variant and — guarded by those
  tests' "no user created" / "invite left unused" assertions — now leave no open transaction.
- **No in-file `#[ignore]` tests** added to `storage/src/sqlite/mod.rs` (it is an
  instrumented dialect file; the SQLite coverage pass skips `#[ignore]` and the PG pass is
  `-p jaunder` only, so an ignored in-file test would run in neither and tank per-file
  coverage). Concurrency/lock regression coverage, if added, belongs in an out-of-dialect
  integration test, not inline.
- **Gate:** `cargo xtask validate --no-e2e` for the per-commit gate; full `cargo xtask
  validate` (sqlite + postgres e2e) before the PR.

## Dependencies / sequencing (ship-time)

Issue #51 references **ADR-0021**, which currently exists *only* in the unmerged branch at
commit `b802c4f`. This worktree is based on `origin/main` (ADRs up to 0020), so:

- The new ADR takes number **0022** to avoid colliding with `b802c4f`'s 0021 when it lands.
- If #51 merges before `b802c4f`, the ADR sequence has a transient **gap at 0021** that
  `b802c4f` fills. The `docs/README.md` ADR table will jump 0020 → 0022 until then.
- This is a reconciliation to confirm at the pre-merge gate, not a code dependency (the
  fix does not import the ADR; cross-references are documentation pointers).

## Out of scope

- `confirm_password_reset` reorder (own follow-up issue).
- Issues #52 (`update_post`) and #53 (`tag_post`) — sibling ADR-0021 follow-ups, separate
  cycles.
- Any change to the SQL statements, error variants, or the Postgres implementation.
