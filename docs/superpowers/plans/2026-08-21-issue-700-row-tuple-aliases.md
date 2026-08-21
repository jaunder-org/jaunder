# Storage Row Tuple Alias Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace meaningful inline storage row tuples with named row aliases or
row structs, and centralize token-state classification without changing storage
behavior.

**Architecture:** Shared token-state and post-row helpers live in
`storage/src/helpers.rs`; module-local row shapes live beside their storage
queries. The token helper classifies a decoded row into a neutral enum, and
callers map that enum into their existing public error types.

**Tech Stack:** Rust, `sqlx::query_as`, storage crate dual-backend tests,
`cargo nextest`, `cargo xtask`.

**Spec:**
[`2026-08-21-issue-700-row-tuple-aliases.md`](../specs/2026-08-21-issue-700-row-tuple-aliases.md)

## Review header

**Scope — in:** named row aliases/structs for the token-state, post
ownership/liveness, tag listing, post tag, audience summary, subscriber summary,
and site-config key/value export rows; one neutral token-state classifier;
focused tests for the classifier; `FromRow` bound updates where generic storage
impls decode renamed rows.

**Scope — out:** one-column tuples, public API shape changes, SQL changes,
transaction changes, claim ordering changes, public error changes, new ADRs, and
mechanical aliasing of every tuple in `storage/src`.

**Tasks:**

1. Add `TokenStateRow` and `TokenState`, test every branch, and route existing
   token error helpers through the classifier.
2. Adopt `TokenStateRow` and the classifier in email verification, password
   reset, invite validation, and atomic password reset.
3. Name post-related row shapes and update generic/backend `FromRow` bounds.
4. Name the selected local summary/export row shapes.

**Key risks / decisions:** `Some((None, expires_at))` is not always the same
semantic state; the classifier must take `now` so invite validation can still
distinguish claimable from expired. Atomic password reset still needs its
rollback reconciliation, so it maps the neutral state locally rather than using
an error-specific helper. Site-config key/value export must keep the public
`Vec<(String, String)>` trait contract while decoding through a named internal
row alias.

## Global Constraints

- Preserve the approved spec exactly: no behavior changes to SQL text,
  transaction boundaries, claim ordering, or public error behavior.
- Follow ADR-0019: every generic storage impl whose decode target changes must
  update the corresponding `FromRow` bound in the same task.
- Follow ADR-0128: do not add item definitions to any `mod.rs`.
- Do not introduce lint suppressions.
- Run focused tests before each commit, then tick the task checkbox, run
  `devtool run -- cargo xtask check`, inspect and stage any mechanical fixes,
  and commit without a `Co-Authored-By` trailer.

---

## File structure

- Modify `storage/src/helpers.rs` — shared token-state row alias, neutral enum,
  classifier, existing claim-error helper mappings, tests, and shared post tag
  row alias if the implementation keeps post tag conversion in helpers.
- Modify `storage/src/email.rs` — decode `TokenStateRow` and pass `now` into the
  email-verification claim error helper.
- Modify `storage/src/password.rs` — decode `TokenStateRow` and pass `now` into
  the password-reset claim error helper.
- Modify `storage/src/postgres/atomic.rs` and `storage/src/sqlite/atomic.rs` —
  decode `TokenStateRow`, use the neutral classifier for invite validation and
  atomic password-reset rejection mapping, and preserve rollback handling.
- Modify `storage/src/posts.rs` — define/adopt `TagListRow` and `PostTagRow`,
  update `post_tags_from_rows`, and update generic bounds for tag listing.
- Modify `storage/src/postgres/posts.rs` and `storage/src/sqlite/posts.rs` —
  decode `PostOwnershipRow` and `PostTagRow`; update backend impl bounds.
- Modify `storage/src/audiences.rs` — define/adopt `AudienceSummaryRow` and
  update the generic `FromRow` bound.
- Modify `storage/src/subscriptions.rs` — define/adopt `SubscriberSummaryRow`
  and update the generic `FromRow` bound.
- Modify `storage/src/site_config.rs` — define/adopt `SiteConfigExportRow`
  internally while preserving
  `SiteConfigStorage::list() -> Vec<(String, String)>`.

## Interfaces and contracts

```rust
// storage/src/helpers.rs
pub(crate) type TokenStateRow = (Option<DateTime<Utc>>, DateTime<Utc>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenState {
    Missing,
    AlreadyUsed,
    Expired,
    Claimable,
}

pub(crate) fn classify_token_state(
    row: Option<TokenStateRow>,
    now: DateTime<Utc>,
) -> TokenState;

pub(crate) fn email_verification_claim_error(
    row: Option<TokenStateRow>,
    now: DateTime<Utc>,
) -> crate::UseEmailVerificationError;

pub(crate) fn password_reset_claim_error(
    row: Option<TokenStateRow>,
    now: DateTime<Utc>,
) -> crate::UsePasswordResetError;
```

```rust
// storage/src/posts.rs or storage/src/helpers.rs, depending on import pressure
pub(crate) type PostOwnershipRow = (UserId, Option<DateTime<Utc>>);
pub(crate) type TagListRow = (TagId, Tag);
pub(crate) type PostTagRow = (PostId, TagId, Tag, TagLabel);
pub(crate) fn post_tags_from_rows(rows: Vec<PostTagRow>) -> Vec<PostTag>;
```

```rust
// Module-local aliases
type AudienceSummaryRow = (AudienceId, AudienceName, DateTime<Utc>);
type SubscriberSummaryRow = (SubscriptionId, String);
type SiteConfigExportRow = (String, String);
```

### Task 1: Add and test the neutral token-state classifier — DONE

**Files:**

- Modify: `storage/src/helpers.rs`
- Test: `storage/src/helpers.rs`

**Interfaces:**

- Consumes: existing `email_verification_claim_error(row)` and
  `password_reset_claim_error(row)` helpers.
- Produces: `TokenStateRow`, `TokenState`, `classify_token_state(row, now)`, and
  updated claim-error helpers taking `now`.

- [x] **Step 1: Write failing helper tests**

Add these unit tests in `storage/src/helpers.rs`'s existing `tests` module:

```rust
#[test]
fn token_state_classifier_distinguishes_all_arms() {
    let now = Utc::now();
    assert_eq!(classify_token_state(None, now), TokenState::Missing);
    assert_eq!(
        classify_token_state(Some((Some(now), now + chrono::Duration::hours(1))), now),
        TokenState::AlreadyUsed
    );
    assert_eq!(
        classify_token_state(Some((None, now)), now),
        TokenState::Expired
    );
    assert_eq!(
        classify_token_state(Some((None, now - chrono::Duration::seconds(1))), now),
        TokenState::Expired
    );
    assert_eq!(
        classify_token_state(Some((None, now + chrono::Duration::seconds(1))), now),
        TokenState::Claimable
    );
}
```

Update the existing claim-error tests to pass `now` and keep their expected
error values unchanged:

```rust
assert_eq!(
    email_verification_claim_error(None, now),
    crate::UseEmailVerificationError::NotFound
);
assert_eq!(
    email_verification_claim_error(Some((Some(now), now)), now),
    crate::UseEmailVerificationError::AlreadyUsed
);
assert_eq!(
    email_verification_claim_error(Some((None, now)), now),
    crate::UseEmailVerificationError::Expired
);
```

Mirror the same shape for `password_reset_claim_error`.

- [x] **Step 2: Run the focused failure**

Run:

```bash
devtool run -- cargo nextest run -p storage token_state_classifier_distinguishes_all_arms
```

Expected: FAIL because `TokenState`, `TokenStateRow`, and `classify_token_state`
are not defined.

- [x] **Step 3: Implement the classifier contract**

In `storage/src/helpers.rs`, add the exact public-in-crate interfaces listed in
this plan. Implement `classify_token_state` with these branches:

- `None` -> `TokenState::Missing`;
- `Some((Some(_), _))` -> `TokenState::AlreadyUsed`;
- `Some((None, expires_at))` with `expires_at <= now` -> `TokenState::Expired`;
- `Some((None, _))` -> `TokenState::Claimable`.

Update the existing claim-error helpers to call `classify_token_state` and map
`Missing`, `AlreadyUsed`, and both `Expired`/`Claimable` to the same public
errors those helpers returned before. `Claimable` maps to `Expired` for these
helpers because they are only called after an atomic claim miss; that mapping
preserves existing behavior if the row is observed with the same `now`.

- [x] **Step 4: Run focused passing tests**

Run:

```bash
devtool run -- cargo nextest run -p storage claim_error_distinguishes_all_arms
devtool run -- cargo nextest run -p storage token_state_classifier_distinguishes_all_arms
```

Expected: PASS.

- [x] **Step 5: Commit the helper classifier**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
```

Inspect and stage the checked tree:

```bash
git add docs/superpowers/specs/2026-08-21-issue-700-row-tuple-aliases.md docs/superpowers/plans/2026-08-21-issue-700-row-tuple-aliases.md storage/src/helpers.rs
git commit -m "refactor(storage): name token state rows (#700)"
```

### Task 2: Adopt token-state rows at all token lookup sites — DONE

**Files:**

- Modify: `storage/src/email.rs`
- Modify: `storage/src/password.rs`
- Modify: `storage/src/postgres/atomic.rs`
- Modify: `storage/src/sqlite/atomic.rs`
- Test: existing token/invite tests in `storage/src/email.rs`,
  `storage/src/password.rs`, and atomic operation coverage through storage
  tests.

**Interfaces:**

- Consumes: `TokenStateRow`, `TokenState`, `classify_token_state`,
  `email_verification_claim_error(row, now)`, and
  `password_reset_claim_error(row, now)` from Task 1.
- Produces: no new interfaces; every inline
  `(Option<DateTime<Utc>>, DateTime<Utc>)` token-state decode target is replaced
  with `TokenStateRow`.

- [x] **Step 1: Replace query decode targets and error-helper calls**

In `storage/src/email.rs` and `storage/src/password.rs`, import or qualify
`crate::helpers::TokenStateRow`, replace each token-state `query_as` target with
`TokenStateRow`, and pass the existing `now` into the claim-error helper.

In both atomic files, replace invite and password-reset token-state decode
targets with `TokenStateRow`. For password reset, replace the inline row match
with `classify_token_state(row, now)` and map:

```rust
let primary = match crate::helpers::classify_token_state(row, now) {
    crate::helpers::TokenState::Missing => Err(ConfirmPasswordResetError::NotFound),
    crate::helpers::TokenState::AlreadyUsed => {
        Err(ConfirmPasswordResetError::AlreadyUsed)
    }
    crate::helpers::TokenState::Expired | crate::helpers::TokenState::Claimable => {
        Err(ConfirmPasswordResetError::Expired)
    }
};
```

Keep `finish_password_reset_rejection(primary, rollback)` unchanged.

For invite validation, classify the fetched row with the current `now` and map:

```rust
match crate::helpers::classify_token_state(row, now) {
    crate::helpers::TokenState::Missing => return Err(RegisterWithInviteError::InviteNotFound),
    crate::helpers::TokenState::AlreadyUsed => {
        return Err(RegisterWithInviteError::InviteAlreadyUsed);
    }
    crate::helpers::TokenState::Expired => return Err(RegisterWithInviteError::InviteExpired),
    crate::helpers::TokenState::Claimable => {}
}
```

Do not change when `now` is taken relative to the existing SQL/transaction steps
except as needed to pass it into the classifier.

- [x] **Step 2: Run focused token/invite storage tests**

Run:

```bash
devtool run -- cargo nextest run -p storage email_verification
devtool run -- cargo nextest run -p storage password_reset
devtool run -- cargo nextest run -p storage invite
```

Expected: PASS. If the test filter misses an existing atomic invite test, run
the nearest existing storage atomic test filter shown by nextest.

- [x] **Step 3: Commit the token-site adoption**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
```

Inspect and stage the checked tree:

```bash
git add docs/superpowers/plans/2026-08-21-issue-700-row-tuple-aliases.md storage/src/email.rs storage/src/password.rs storage/src/postgres/atomic.rs storage/src/sqlite/atomic.rs storage/src/helpers.rs
git commit -m "refactor(storage): classify token state rows centrally (#700)"
```

### Task 3: Name post row shapes and update `FromRow` bounds

**Files:**

- Modify: `storage/src/helpers.rs` or `storage/src/posts.rs`
- Modify: `storage/src/posts.rs`
- Modify: `storage/src/postgres/posts.rs`
- Modify: `storage/src/sqlite/posts.rs`
- Test: existing post/tag storage tests in `storage/src/posts.rs`

**Interfaces:**

- Consumes: existing post update and tag query code.
- Produces: `PostOwnershipRow = (UserId, Option<DateTime<Utc>>)`,
  `TagListRow = (TagId, Tag)`, `PostTagRow = (PostId, TagId, Tag, TagLabel)`,
  and `post_tags_from_rows(rows: Vec<PostTagRow>) -> Vec<PostTag>`.

- [ ] **Step 1: Name the post rows**

Define the row aliases where the import graph stays simplest:

- `PostOwnershipRow` must be reachable from both backend-specific `posts.rs`
  files;
- `PostTagRow` must be reachable from both backend-specific `posts.rs` files and
  the `post_tags_from_rows` helper;
- `TagListRow` can stay in `storage/src/posts.rs`.

Replace inline `query_as` tuple targets at:

- `storage/src/postgres/posts.rs` `update_post` ownership/liveness query;
- `storage/src/sqlite/posts.rs` `update_post` ownership/liveness query;
- both `SELECT_POST_TAGS` query sites;
- both `list_tags` query variants in `storage/src/posts.rs`.

Update every affected generic or backend impl bound from the raw tuple to the
new alias.

- [ ] **Step 2: Run focused post/tag tests**

Run:

```bash
devtool run -- cargo nextest run -p storage posts
```

Expected: PASS.

- [ ] **Step 3: Commit the post row aliases**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
```

Inspect and stage the checked tree:

```bash
git add docs/superpowers/plans/2026-08-21-issue-700-row-tuple-aliases.md storage/src/helpers.rs storage/src/posts.rs storage/src/postgres/posts.rs storage/src/sqlite/posts.rs
git commit -m "refactor(storage): name post row tuples (#700)"
```

### Task 4: Name selected local summary/export row shapes

**Files:**

- Modify: `storage/src/audiences.rs`
- Modify: `storage/src/subscriptions.rs`
- Modify: `storage/src/site_config.rs`
- Test: existing tests in those files.

**Interfaces:**

- Consumes: no interfaces from earlier tasks.
- Produces: `AudienceSummaryRow = (AudienceId, AudienceName, DateTime<Utc>)`,
  `SubscriberSummaryRow = (SubscriptionId, String)`, and
  `SiteConfigExportRow = (String, String)`.

- [ ] **Step 1: Add local aliases and replace decode targets**

In `storage/src/audiences.rs`, add a private `AudienceSummaryRow` alias near
`AudienceRecord`, decode `list_audiences` through it, and update the generic
`FromRow` bound.

In `storage/src/subscriptions.rs`, add a private `SubscriberSummaryRow` alias
near `SubscriberSummaryRecord`, decode `list_subscriber_summaries` through it,
and update the generic `FromRow` bound.

In `storage/src/site_config.rs`, add a private `SiteConfigExportRow` alias near
the `SiteConfigStorage` impl, decode `list` through it, and keep the trait and
method return type as `Vec<(String, String)>`.

- [ ] **Step 2: Run focused summary/export tests**

Run:

```bash
devtool run -- cargo nextest run -p storage audiences
devtool run -- cargo nextest run -p storage subscriptions
devtool run -- cargo nextest run -p storage site_config_primitives_round_trip
```

Expected: PASS.

- [ ] **Step 3: Run the type-safety population check**

Run:

```bash
devtool run -- rg "query_as::<_, \\((Option<DateTime<Utc>>, DateTime<Utc>)|(UserId, Option<DateTime<Utc>>)|(TagId, Tag)|(PostId, TagId, Tag, TagLabel)|(AudienceId, AudienceName, DateTime<Utc>)|(SubscriptionId, String)|(String, String))\\)" storage/src
```

Expected: exit 1 with `stdout.lines = 0`. For this check, `ok:false` is the
success proof because `rg` exits 1 on no matches; read the parked stdout path if
needed to confirm it is empty. Remaining one-column tuples such as `(i64,)`,
`(String,)`, `(UserId,)`, and typed SMTP value reads are expected and are out of
scope.

- [ ] **Step 4: Commit the local row aliases**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
```

Inspect and stage the checked tree:

```bash
git add docs/superpowers/plans/2026-08-21-issue-700-row-tuple-aliases.md storage/src/audiences.rs storage/src/subscriptions.rs storage/src/site_config.rs
git commit -m "refactor(storage): name summary row tuples (#700)"
```

## Final verification before ship

After all tasks are committed, run:

```bash
devtool run -- cargo xtask validate --no-e2e
```

Expected: PASS. Use `jaunder-ship` for the final review, PR, CI monitoring, and
merge-gate handoff.
