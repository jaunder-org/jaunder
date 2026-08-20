# #697 Newtype Conversion Closure Implementation Plan

**Issue:** [#697](https://github.com/jaunder-org/jaunder/issues/697) **Spec:**
`/home/mdorman/src/jaunder/agent-6/docs/superpowers/specs/2026-08-20-issue-697-newtype-adoption-gate.md`

## For agentic workers

Execute this plan task-by-task with `jaunder-iterate` (delegating individual
tasks to `jaunder-dispatch` when useful). Scope stays conversion closure: no
broad adoption gate, no central registry, no marker contract system.

## Global constraints

- Preserve product behavior and wire formats.
- Prefer existing `FromStr`/`TryFrom`/newtype APIs; do not add permissive
  constructors.
- Convert only seams whose complete domain value is already owned by an existing
  type.
- Leave deliberate primitives in place with a local rationale when semantics are
  ambiguous or intentionally raw.
- File follow-up issues for design-sized candidates instead of solving them in
  this cycle.
- Before each commit: tick the matching checkbox in this plan, run the listed
  focused check, inspect/stage mechanical formatter output, then commit.
- Commit command shape: stage the plan file and the exact source/test files
  changed by the task, then run the task-specific `git commit -m ...` command
  shown below. If a task only files follow-up issues and leaves no tracked file
  changes, record the issue URLs in the final handoff instead of making an empty
  commit.

## Checklist

- [x] Task 1 — Reconstruct and classify remaining primitive seams
- [x] Task 2 — Convert Basic auth token to `RawToken`
- [x] Task 3 — Convert CLI parse-time complete values
- [x] Task 4 — Tighten AtomPub service and collection cursors
- [x] Task 5 — Convert media upload size plumbing to `ByteSize`
- [ ] Task 6 — Document deliberately retained primitives
- [ ] Task 7 — File follow-up issues for design-sized seams
- [ ] Task 8 — Run final gate and prepare for ship

## Task 1 — Reconstruct and classify remaining primitive seams

**Files / interfaces**

- `docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md`
- `docs/superpowers/specs/2026-08-20-issue-697-newtype-adoption-gate.md`
- Production roots: `common/src`, `web/src`, `storage/src`, `server/src`,
  `host/src`, `client/src`
- Issue evidence: `issue://jaunder-org/jaunder/697`
- ADR evidence: `docs/adr/0063-domain-value-newtype-convention.md`

**Work**

- Re-run a source inspection over production roots for complete domain values
  still represented by `String`, `&str`, integer primitives, or boolean flags.
- Classify each candidate as one of:
  - `convert`: existing newtype owns the full value and conversion is
    local/mechanical.
  - `already typed`: current code already uses a domain newtype.
  - `deliberate primitive`: semantics are intentionally raw, ambiguous, or
    outside a complete domain value.
  - `follow-up`: candidate needs a new domain type, behavior decision, or
    route/storage compatibility design.
- Update this plan if the live tree differs from the current expected candidate
  set:
  - Convert: Basic auth token, CLI password/email args, AtomPub service
    `accept`, AtomPub `updated_before`, media upload byte-size plumbing.
  - Deliberate primitive: tag suggestion `prefix`, client local-storage `key`,
    test/open-ended helper strings, constants with non-domain byte-unit values.
  - Follow-up likely outside this cycle: post idempotency key, subscriber
    reference, proxy/cached remote media URL, feed cache body, public path/date
    extension wrappers.

**Task notes**

- `convert`: `common/src/auth.rs` + `host/src/auth.rs` Basic-auth token;
  `server/src/cli.rs` + `server/src/commands.rs` CLI password/email;
  `common/src/atompub/service.rs`, `server/src/atompub/service.rs`, and
  `server/src/atompub/posts.rs` AtomPub `accept`/`updated_before`;
  `storage/src/media_manager.rs` media upload `size_bytes`.
- `already typed`: `web/src/auth/api.rs` session-label login seam;
  `web/src/tags/api.rs` page-size limit; `common/src/feed/metadata.rs` feed
  metadata/title/summary/html/tag fields; `common/src/ids.rs` row IDs;
  `storage/src/media.rs` media records; `storage/src/site_config.rs` typed
  config accessors.
- `deliberate primitive`: `web/src/tags/api.rs` partial `prefix`;
  `client/src/storage.rs` browser `localStorage` key/value selectors;
  `common/src/tag.rs` local cardinality counts; `server/src/atompub/posts.rs`
  public-query `limit` clamp; `server/src/media.rs` raw hash-prefix path
  segments; `server/src/soft_path.rs` soft public route parsing;
  `storage/src/sqlite/backup.rs` `serde_json::Value` wire-decoder carve-out.
- `follow-up`: post idempotency key (`web/src/posts/api.rs`,
  `storage/src/post_service.rs`, `storage/src/posts.rs`); subscriber reference
  (`storage/src/subscriptions.rs`, `common/src/visibility.rs`, existing #750);
  proxy/cached remote media URL (`server/src/media.rs`); feed cache body
  (`storage/src/feed_cache.rs`); public path/date extension wrappers
  (`server/src/feed/handlers.rs`, `server/src/projector/handlers.rs`).

**Check**

- No validation command. Evidence is the updated plan classification and exact
  paths.

**Commit**

```bash
git add docs/superpowers/specs/2026-08-20-issue-697-newtype-adoption-gate.md docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "docs(types): classify issue 697 conversion seams"
```

## Task 2 — Convert Basic auth token to `RawToken`

**Files / interfaces**

- `common/src/auth.rs`
- `host/src/auth.rs`

**Work**

- Change `parse_basic_auth` to return `Option<(Username, RawToken)>`.
- Parse the password segment as `RawToken` inside `parse_basic_auth`; malformed
  token shape returns `None`.
- Update `host::auth::resolve_credential` to consume the parsed `RawToken`
  directly, removing the duplicate `RawToken::from_str(&password)` conversion.
- Update unit tests:
  - `parse_basic_auth_decodes_credentials` asserts the token equals
    `RawToken::from_str("tok123").unwrap()`.
  - Add `parse_basic_auth_rejects_invalid_token` using a Basic credential whose
    password contains a space or other invalid token character.

**Check**

```bash
devtool run -- cargo nextest run -p common parse_basic_auth
```

**Commit**

```bash
git add common/src/auth.rs host/src/auth.rs docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "fix(auth): return raw token from basic auth parser"
```

## Task 3 — Convert CLI parse-time complete values

**Files / interfaces**

- `server/src/cli.rs`
- `server/src/commands.rs`

**Work**

- Change `Commands::UserCreate.password` from `Option<String>` to
  `Option<Password>`.
- Remove `parse_password`; prompt parsing remains in the interactive password
  path because it starts from terminal text.
- Pass `password.as_ref()` or equivalent into `cmd_user_create` without
  reparsing.
- Change `Commands::SmtpTest.to` from `String` to `Email`.
- Pass `&Email` into `cmd_smtp_test`; remove the late `to.parse::<Email>()`
  branch.
- Update CLI tests:
  - `user_create_parses_username_and_password` expects a parsed `Password` via a
    test helper such as `parse_cli_password("secret123")`.
  - `user_create_malformed_password_is_clap_error` proves empty/invalid password
    is rejected by clap before handlers run.
  - `smtp_test_parses_to` expects `parse_email("alice@example.com")`.
  - `smtp_test_malformed_to_is_clap_error` proves invalid email is rejected by
    clap before SMTP setup.

**Check**

```bash
devtool run -- cargo nextest run -p jaunder cli::tests::user_create
```

```bash
devtool run -- cargo nextest run -p jaunder cli::tests::smtp_test
```

**Commit**

```bash
git add server/src/cli.rs server/src/commands.rs server/src/main.rs server/tests/misc/commands.rs docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "fix(cli): parse command values into domain types"
```

## Task 4 — Tighten AtomPub service and collection cursors

**Files / interfaces**

- `common/src/atompub/service.rs`
- `server/src/atompub/service.rs`
- `server/src/atompub/posts.rs`

**Work**

- Change `CollectionDecl.accept` from `Vec<String>` to `Vec<ContentType>`.
- Build service document accept values through `ContentType::from_str` or a
  local helper whose fixtures assert the literals are valid.
- Keep serialization output identical by writing `accept.as_ref()`.
- Change `CollectionPaging.updated_before` from `Option<String>` to
  `Option<UtcInstant>`.
- In `collection`, remove manual RFC 3339 parsing and use
  `DateTime::<Utc>::from(updated_before)` or `updated_before.value()` when
  building `CollectionCursor`.
- Add or update tests proving:
  - service document output still contains `application/atom+xml;type=entry` and
    media accept strings;
  - malformed `updated_before` query returns `400 Bad Request` through the
    existing Axum query path, not a handler-internal parse branch.

**Check**

```bash
devtool run -- cargo nextest run -p common service_document
```

```bash
devtool run -- cargo nextest run -p jaunder atompub
```

**Commit**

```bash
git add common/src/atompub/service.rs server/src/atompub/service.rs server/src/atompub/posts.rs docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "fix(atompub): carry service metadata as domain values"
```

## Task 5 — Convert media upload size plumbing to `ByteSize`

**Files / interfaces**

- `storage/src/media_manager.rs`
- `web/src/media/api.rs`

**Work**

- Change `UploadMetadata.size_bytes` from `i64` to `ByteSize`.
- Convert from the stream/write byte count to `ByteSize` at the ingestion
  boundary, before quota/database/final response assembly.
- Change `MediaManager::check_quota` and `register_in_db` parameters to take
  `ByteSize` where they mean a validated byte count.
- Remove the repeated `ByteSize::try_from(metadata.size_bytes)` in
  `finalize_upload`.
- Keep metric emission exact: convert `ByteSize` to a primitive only at the
  metric boundary.
- Update existing `media_manager` tests and add a focused assertion that
  `UploadMetadata`/`register_in_db` rejects or cannot construct a negative byte
  count before database insertion.

**Check**

```bash
devtool run -- cargo nextest run -p storage media_manager
```

```bash
devtool run -- cargo nextest run -p web media
```

**Commit**

```bash
git add storage/src/media_manager.rs web/src/media/api.rs docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "fix(media): carry upload sizes as byte values"
```

## Task 6 — Document deliberately retained primitives

**Files / interfaces**

- `web/src/tags/api.rs`
- `client/src/storage.rs`
- Any additional deliberate primitive sites found by Task 1
- `docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md`

**Work**

- For every candidate classified as `deliberate primitive`, verify the source
  has a local rationale explaining why a domain newtype would be wrong or
  premature.
- Add short comments only where the rationale is missing. Expected sites:
  - `web::tags::api::list(prefix: Option<String>, ...)`: `prefix` is a partial
    search fragment, not a complete `Tag`.
  - `client::storage::{get,set,remove}(key: &str, ...)`: browser `localStorage`
    keys are open-ended infrastructure selectors, not a Jaunder domain value.
  - non-domain unit constants and generated/test-only helper strings: leave as
    primitives because they are not persisted/user domain values.
- Record any additional retained primitive site in this plan with its source
  rationale.

**Check**

- Source inspection shows each deliberate primitive retained by the cycle has a
  nearby rationale comment or is listed in this plan as test/helper-only.

**Commit**

```bash
git add web/src/tags/api.rs client/src/storage.rs docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "docs(types): explain retained primitive seams"
```

## Task 7 — File follow-up issues for design-sized seams

**Files / interfaces**

- GitHub issues in `jaunder-org/jaunder`
- `docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md`

**Work**

- Search open `jaunder-org/jaunder` issues for each `follow-up` candidate before
  creating anything; link an existing issue instead of filing a duplicate.
- Known overlap from planning review: #750 already covers the `SubscriberRef`
  channel-scoped subscriber reference. Link it in this plan; do not create a
  second subscriber-reference issue.
- For each remaining `follow-up` candidate still present after Tasks 2–5, create
  a GitHub issue with:
  - exact current primitive seam;
  - why the value has domain semantics;
  - why conversion is not mechanical in this cycle;
  - relation to issue #697 and ADR-0063.
- Expected follow-ups unless source inspection or issue search disproves them:
  - post idempotency key;
  - subscriber reference: existing issue #750;
  - proxy/cached remote media URL;
  - feed cache body;
  - public path/date extension wrappers.
- Link existing and created issue URLs in this plan under the task notes.

**Check**

- Read every linked issue URL and verify the body names #697 plus ADR-0063 and
  has the exact path/symbol seam, or update it with the missing relationship.

**Commit**

```bash
git add docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md
git commit -m "docs(types): link issue 697 follow-ups"
```

## Task 8 — Run final gate and prepare for ship

**Files / interfaces**

- Full working tree
- `docs/superpowers/plans/2026-08-20-issue-697-newtype-conversion-closure.md`

**Work**

- Confirm every plan task checkbox is checked.
- Run the repo check gate.
- Inspect and stage any formatter/mechanical changes from `cargo xtask check`.
- Commit gate-induced mechanical changes if present.
- Leave branch ready for `jaunder-ship`.

**Check**

```bash
devtool run -- cargo xtask check
```

**Commit**

- If the gate changed tracked files, stage the exact gate-touched files plus
  this plan and commit with:

```bash
git commit -m "chore: apply issue 697 gate fixes"
```

If no gate-touched files exist, make no commit.
