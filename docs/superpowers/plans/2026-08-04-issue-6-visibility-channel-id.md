# Plan — issue #6: the author branch must fire only for a local viewer

**Spec:** `docs/superpowers/specs/2026-08-04-issue-6-visibility-channel-id.md`
(the "what" and "why" live there; this plan is the "how"). **Issue:**
[#6](https://github.com/jaunder-org/jaunder/issues/6). **Branch:**
`worktree-issue-6-visibility-channel-id`. **Fork-point tag:** `wt-base-issue-6`.

**For agentic workers:** drive execution with **`jaunder-iterate`**, delegating
an individual task to a subagent via **`jaunder-dispatch`** where useful. Tick
checkboxes in real time.

## Review header

**Goal.** Make "the author sees their own post" fire only for a _local_ viewer,
by making locality a type-level fact rather than an unchecked assumption about a
string — closing the privacy hole before Layer B can construct a remote viewer.

**Scope — in:** `ViewerIdentity`'s shape; `resolution_where` +
`ResolutionBinds`; `viewer_user_id`; `is_subscriber`; the consequential removals
the field drop forces (`account_viewer`, the `LOCAL_CHANNEL_ID` OnceLock,
`is_subscribed`'s channel lookup, `post_service`'s `ChannelId::from(0)`, the
AtomPub `SubscriptionStorage` DI seam); ADR-0020 and ADR-0063 corrections.

**Scope — out:** constructing any `Remote` viewer in production (Layer B/C);
issue #342 itself; validating a `Remote`'s `channel_id` against `channels`. **No
separable concerns need filing as issues** — the spec's one deferred item
(collapsing the `Anonymous` arm) was declined as a micro-optimization, not a
defect, and #342 gets a ship-step comment rather than a new issue. So there is
no issue-filing task.

**Tasks.**

1. Split `ViewerIdentity` into `Local { user_id, channel_id }` + `Remote{…}`;
   author branch fires on `Local` only. **This is the security fix** — AC2, AC4,
   AC5 go green here, independently of everything below.
2. `viewer_user_id` returns `Some` only for `Local` — AC6.
3. Resolve the local channel in SQL and make `ResolutionBinds` variant-shaped —
   done _while_ `Local` still carries `channel_id`, so it gates on its own. AC3,
   AC9.
4. Drop `channel_id` from `Local`. Compiler-forced mechanical fan-out, including
   the whole AtomPub parameter cascade — AC1, AC7, AC12, most of AC10.
5. Lock the post-create re-read with a non-public-targeting test — AC7.
6. Delete the orphaned `LOCAL_CHANNEL_ID` OnceLock, free fn, and its test — AC8.
7. Remove the AtomPub router extension (`server/src/lib.rs` only) — AC10.
8. Correct ADR-0020, ADR-0063, and the stale doc comments — AC11.
9. Full gate, then `server-fn-coverage` reconciliation — AC13.

**Key risks / decisions.**

- **Task 4 is unavoidably large, and larger than it first looks.** Removing a
  field from a widely-destructured enum is a type-driven refactor, and under
  `-D warnings` (`xtask/src/steps/static_checks.rs:56` runs
  `cargo clippy --all-targets -- -D warnings`) the cascade does not stop at the
  destructuring sites: once `owner_viewer` drops its `subscriptions` parameter,
  `owned_post`'s becomes unused, then the handlers' `Extension(subscriptions)`,
  then `PostServices.subscriptions` — every one a hard gate failure. All of that
  is one commit. Tasks 1-3 exist to keep the _interesting_ work out of it.
- **Variable bind arity.** Task 3 changes `resolution_where`'s emitted
  placeholder count per variant. Verified safe at all 14 call sites (the 10 with
  trailing binds thread the returned `next`; the 4 that discard it, and
  `window_sql`, place the fragment last). Any new call site must thread `next` —
  do not hard-code `$n+5`.
- **`is_subscriber` is const-based, in two dialects** (ADR-0019). Task 3 adds
  `IS_ACTIVE_LOCAL_SUBSCRIBER` to _both_ `storage/src/sqlite/` and
  `storage/src/postgres/` — `?` vs `$n` placeholders. Editing only one backend
  passes SQLite tests and fails Postgres.
- **Task 2's red depends on task 1 not fixing it early.** `viewer_user_id`
  destructures the variant task 1 deletes, so task 1 must rewrite it — and the
  _obvious_ rewrite already satisfies task 2's test. Task 1 therefore pins it to
  the parse form deliberately (step 3 below).
- **`#[server]` flow coverage** is byte-for-byte gated and its `regenerate`
  reads an e2e capture that must already exist (`xtask/src/lib.rs:623-629`) — so
  task 9 runs `validate` **first**, then regenerates only if the snapshot moved.
  The shift is unlikely: `viewer_identity` is not a `#[server]` fn and
  `is_subscribed` survives as one.

## Global constraints

- **Every commit is gated.** Run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-6-visibility-channel-id -- cargo xtask check`
  and get it green **before** `git commit` (the pre-commit hook runs the same
  thing). Stage, then commit — never `git commit -- <paths>`. See
  **`jaunder-commit`**.
- **No `Co-Authored-By` trailer.**
- **Storage tests use the dual-backend template** — `#[apply(backends)]` taking
  `backend: Backend`. A bare `#[tokio::test]` on a storage test trips the
  `test-backend-pattern` guard.
- **Never put tests in ADR-0019 per-backend dialect files**
  (`storage/src/{sqlite,postgres}/*.rs`).
- Commit messages: `fix(...)`, `refactor(...)`, `test(...)`, `docs(...)`, each
  referencing `(#6)`.

---

## Task 1 — split the variant; the author branch fires on `Local` only

**Files**

- `common/src/visibility.rs` — the enum, the `local` constructor,
  `account_viewer`, `viewer_user_id`.
- `storage/src/posts.rs` — `resolution_where`'s match (`:1976-1988`).
- `storage/src/subscriptions.rs` — `is_subscriber`'s match (`:208`).
- `server/tests/storage/mod.rs` — `resolution_matrix` (`:5707`).

**Interfaces**

```rust
// common/src/visibility.rs
pub enum ViewerIdentity {
    Anonymous,
    /// A logged-in local account. Locality is carried by the *variant*: the
    /// author branch of the resolution filter fires on this and nothing else.
    Local { user_id: UserId, channel_id: ChannelId },
    /// A non-local channel identity (ActivityPub actor, email address).
    Remote { channel_id: ChannelId, subscriber_ref: String },
}
```

`ViewerIdentity::local(user_id, local_channel_id)` keeps its two-argument shape
in this task and constructs `Local`. `account_viewer` is unchanged.

**Steps**

- [ ] **Step 1 (red).** In `server/tests/storage/mod.rs`, seed a second channel
      in `resolution_matrix` and add the adversarial viewer. Note this is
      **not** a drop-in addition: the expectation rows are typed
      `&[(&str, PostId, [bool; 5])]` (`:5768`) and the viewer array
      `[(&str, &ViewerIdentity); 5]` (`:5784`) — widen all six rows to
      `[bool; 6]` and the array to `; 6`.

  ```rust
  // A non-local channel: the shape Layer B will produce.
  let remote_channel = /* INSERT INTO channels (name) VALUES ('activitypub'),
                          then read back its channel_id, per backend */;
  // The adversarial case: a remote ref that is the decimal form of the
  // author's local user id. Must NOT be treated as the author (issue #6).
  let impostor = ViewerIdentity::Remote {
      channel_id: remote_channel,
      subscriber_ref: author.to_string(),
  };
  ```

  Expectations for the impostor column: **Public** visible; **Private**,
  **Subscribers**, **Named(G)**, **Named(G2)** not — asserted through both
  `get_post_by_id` and presence in `list_published`.

- [ ] **Step 2 (still red, by construction).** Add the `Local`/`Remote` variants
      and update every destructuring/construction site so the tree compiles, but
      keep `resolution_where` deriving `author_id` **by the parse**, on both
      arms. `Local` has no `subscriber_ref` field, so its arm synthesizes one —
      exactly what `local()` stores today (`common/src/visibility.rs:89-94`), so
      behavior is byte-identical to `main`:

  ```rust
  // Deliberately still channel-blind — this is the bug, made observable.
  ViewerIdentity::Local { user_id, channel_id } => {
      let subref = user_id.to_string();
      (subref.parse::<UserId>().ok(), Some(*channel_id), Some(subref))
  }
  ViewerIdentity::Remote { channel_id, subscriber_ref } => (
      subscriber_ref.parse::<UserId>().ok(),
      Some(*channel_id),
      Some(subscriber_ref.clone()),
  )
  ```

  Run `cargo nextest run -p jaunder --test storage resolution_matrix` → **FAIL**
  on the impostor's Private/Subscribers/Named cells.

- [ ] **Step 3 (keep task 2's red).** `viewer_user_id`
      (`common/src/visibility.rs:118-123`) also destructures the deleted variant
      and must be rewritten now — but rewrite it to the **parse form**, not the
      variant match:

  ```rust
  // Task 2 replaces this with a variant match; keep the parse here so that
  // task 2's regression test starts red.
  ViewerIdentity::Local { user_id, .. } => user_id.to_string().parse().ok(),
  ViewerIdentity::Remote { subscriber_ref, .. } => subscriber_ref.parse().ok(),
  ```

- [ ] **Step 4 (green).** Change `resolution_where`'s match so `author_id` is
      `Some(*user_id)` for `Local` and `None` for `Remote` and `Anonymous`:

  ```rust
  let (author_id, channel, subref) = match viewer {
      ViewerIdentity::Anonymous => (None, None, None),
      ViewerIdentity::Local { user_id, channel_id } => (
          Some(*user_id), Some(*channel_id), Some(user_id.to_string()),
      ),
      // A remote viewer is never the author, whatever its ref looks like.
      ViewerIdentity::Remote { channel_id, subscriber_ref } => (
          None, Some(*channel_id), Some(subscriber_ref.clone()),
      ),
  };
  ```

  Re-run → **PASS**, both backends. The pre-existing matrix cells (AC5) must be
  untouched: every existing viewer is `Anonymous` or `local(x, local)`
  (`:5761-5790`), for which `Some(*user_id)` is exactly what the parse yielded.

- [ ] **Step 5.** `cargo xtask check`; commit
      `fix(visibility): author branch fires only for a local viewer (#6)`.

---

## Task 2 — `viewer_user_id` is not spoofable by a numeric ref

**Files:** `common/src/visibility.rs` (fn + its `#[cfg(test)]` module).

**Steps**

- [ ] **Step 1 (red).** Add a unit test:

  ```rust
  #[test]
  fn viewer_user_id_is_none_for_a_remote_viewer_with_a_numeric_ref() {
      let impostor = ViewerIdentity::Remote {
          channel_id: ChannelId::from(2),
          subscriber_ref: "42".to_owned(),
      };
      assert_eq!(viewer_user_id(&impostor), None);
  }
  ```

  `cargo nextest run -p common viewer_user_id` → **FAIL** (task 1 step 3 left
  the parse in place precisely so this is red).

- [ ] **Step 2 (green).** Match on the variant instead of parsing:
      `Local { user_id, .. } => Some(*user_id)`,
      `Remote { .. } | Anonymous =>     None`. Re-run → **PASS**.
- [ ] **Step 3.** `cargo xtask check`; commit
      `fix(visibility): is_author cannot be spoofed by a numeric remote ref (#6)`.

---

## Task 3 — resolve the local channel in SQL; `ResolutionBinds` becomes an enum

Doable **before** the field drop: match `Local { user_id, .. }` and ignore the
carried `channel_id`. For a local viewer that id _is_ the local row, so the
subquery is semantically identical — which is what makes this an independently
gated commit rather than part of the fan-out.

**Files**

- `storage/src/posts.rs` — `resolution_where` (`:1974`), `ResolutionBinds`
  (`:1933-1944`), `bind_onto` (`:2030-2052`).
- `storage/src/subscriptions.rs` — `is_subscriber`'s `Local` arm.
- `storage/src/sqlite/subscriptions.rs`, `storage/src/postgres/subscriptions.rs`
  — the new const, **both dialects**.

**Interfaces**

```rust
// storage/src/posts.rs — arity is per-variant, so mirror the variant.
enum ResolutionBinds {
    Anonymous,                                       // 5 NULL binds
    Local { user_id: UserId, subref: String },       // 3 binds
    Remote { channel: ChannelId, subref: String },   // 5 binds
}
```

```sql
-- storage/src/sqlite/subscriptions.rs (`?`) and
-- storage/src/postgres/subscriptions.rs (`$n`) — same query, two dialects.
IS_ACTIVE_LOCAL_SUBSCRIBER = "SELECT EXISTS( \
    SELECT 1 FROM subscriptions s \
    JOIN subscription_statuses st ON st.status_id = s.status_id \
    WHERE s.author_user_id = ? \
      AND s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local') \
      AND s.subscriber_ref = ? \
      AND st.name = 'active')"
```

**Steps**

- [ ] **Step 1.** In `resolution_where`, emit the channel predicate per variant.
      The `subscribers` and `named` branches take an interpolated
      **expression**, not a fixed placeholder:

  ```rust
  let channel_pred = match viewer {
      ViewerIdentity::Local { .. } =>
          "(SELECT channel_id FROM channels WHERE name = 'local')".to_owned(),
      _ => format!("${sub_channel}"),
  };
  ```

  Return `start + 3` for `Local`, `start + 5` otherwise. Convert
  `ResolutionBinds` to the enum above; `bind_onto` matches on it. **Do not
  hard-code any downstream placeholder number** — the 10 call sites with
  trailing binds already thread the returned `next`; verify each still does.

- [ ] **Step 2.** Add `IS_ACTIVE_LOCAL_SUBSCRIBER` to **both** backend modules
      and route `is_subscriber`'s `Local` arm to it (2 binds: author, ref).
      `Remote` keeps `IS_ACTIVE_SUBSCRIBER`; `Anonymous` keeps short-circuiting.
- [ ] **Step 3.** `cargo nextest run -p jaunder --test storage` → **PASS**, both
      backends — the full matrix including task 1's impostor column, plus the
      `is_subscriber` tests at `:293-390`.
- [ ] **Step 4.** `cargo xtask check`; commit
      `refactor(storage): resolve the local channel in SQL (#6)`.

---

## Task 4 — drop `channel_id` from `Local`

One commit: the tree does not build midway.

**Files**

- `common/src/visibility.rs` — enum, `local(user_id)`, **delete**
  `account_viewer`, update its unit tests (`:379-419`).
- `storage/src/post_service.rs:526` — the `ChannelId::from(0)` placeholder.
- `web/src/viewer.rs` — construct `Local { user_id }` directly; the module doc
  at `:7-15, 28-30, 39-41` names `ViewerIdentity::Channel`, `account_viewer`,
  `local_channel_id` and a fail-closed contract that no longer exists — rewrite
  it here rather than deferring (task 8 will not need to revisit it).
- `web/src/subscriptions/api.rs:65-67` — drop `is_subscribed`'s channel lookup.
- `web/src/posts/api.rs:935-951` — the `MockSubscriptionStorage` /
  `expect_local_channel_id` in `mutation_owner`, **and** the imports it strands.
- `server/src/atompub/posts.rs` — the full cascade, all compiler-forced under
  `-D warnings`: `owner_viewer` (`:219-225`) becomes infallible and loses
  `subscriptions`; `owned_post` (`:234`) loses its now-unused parameter; the
  handler `Extension(subscriptions)` params (`:261`, `:298`) go; the
  `PostServices.subscriptions` field (`:43`, extracted `:56`, destructured
  `:338`, `:461`) goes; pass-through uses at `:268, 305, 388, 467, 517` follow.
- Tests: `server/tests/storage/mod.rs` (`:307, 330, 390, 5762-5765`),
  `server/tests/web/web_subscriptions.rs` (`:20, 77` — **and** the now-unused
  `let channel = …` at `:19, 64`), `server/tests/web/web_posts.rs` (`:2624` —
  and the now-unused `let local = …` at `:2513`),
  `server/tests/misc/backup_fixture.rs` (`:190, 211`).

**Interfaces**

```rust
pub enum ViewerIdentity {
    Anonymous,
    Local { user_id: UserId },
    Remote { channel_id: ChannelId, subscriber_ref: String },
}
```

**Steps**

- [ ] **Step 1.** Drop the field; `local(user_id)` takes one argument; delete
      `account_viewer`. `post_service.rs:526` becomes
      `ViewerIdentity::Local { user_id }` — delete the now-false comment about
      `0` being a harmless placeholder.
- [ ] **Step 2.** Work the compile errors outward through the file list above.
      Expect a second wave from `-D warnings` (unused params, bindings, imports)
      after the first wave of type errors clears — `cargo clippy --all-targets`
      is the check that matters, not `cargo build`.
- [ ] **Step 3.** Update `common/src/visibility.rs`'s unit tests: the two
      `account_viewer_*` tests lose their subject and are removed; keep coverage
      of the `local` constructor and `viewer_user_id`'s three arms so the
      coverage gate does not regress.
- [ ] **Step 4.** `cargo nextest run -p jaunder --test storage`, `--test web`,
      and the atompub suite → **PASS**, both backends.
- [ ] **Step 5.** `cargo xtask check`; commit
      `refactor(visibility): a local viewer is just a user id (#6)`.

---

## Task 5 — lock the post-create re-read against non-public targeting

The existing `perform_post_creation` tests all pass `AudienceTarget::Public` (20
occurrences, `storage/src/post_service.rs:581 … :1115`), so none can observe
that the re-read succeeds for a post its author alone can see. There is no
audience validation — `render_post_input` (`:67-92`) passes `audiences` straight
through and `audience_target_row` (`storage/src/posts.rs:2057`) emits no row for
an absent target — so `vec![]` is genuinely the Private case, as
`resolution_matrix` itself already assumes (`server/tests/storage/mod.rs:3743`).

**Files:** `storage/src/post_service.rs` (`#[cfg(test)]` module).

**Steps**

- [ ] **Step 1.** Add a dual-backend test creating a post with
      `audiences: vec![]` via `perform_post_creation`, asserting the returned
      record's `post_id`/`slug` match what was created — i.e. the author re-read
      still resolves. Model it on the nearest existing create test.
- [ ] **Step 2.** `cargo nextest run -p storage perform_post_creation` →
      **PASS**. This is a lock-in test, not a red/green: task 4 already made the
      behavior correct; this stops a future change from silently breaking it.
- [ ] **Step 3.** `cargo xtask check`; commit
      `test(storage): post-create re-read resolves for a private post (#6)`.

---

## Task 6 — delete the orphaned process-global channel cache

**Files:** `storage/src/subscriptions.rs` (`:73-75` doc, `:84` static, `:94-102`
free fn, `:266-285` test).

**Steps**

- [ ] **Step 1.** Confirm no production caller remains:
      `rg -n 'local_channel_id' --glob '!target'` — the only survivors must be
      the **trait** method, its two impls, `web/src/subscriptions/api.rs:27,43`
      (subscribe/unsubscribe — the write path), and tests.
- [ ] **Step 2.** Delete the free fn, the `LOCAL_CHANNEL_ID` static, and the
      test `local_channel_id_returns_the_seeded_local_channel`. Reword the trait
      method's doc comment (`:73-75`), which points at the deleted fn and claims
      the result is memoized per process.
- [ ] **Step 3.** `cargo xtask check`; commit
      `refactor(storage): drop the orphaned local-channel OnceLock (#6)`.

---

## Task 7 — remove the AtomPub `SubscriptionStorage` router extension

The handler-side cascade landed in task 4; this is the router plumbing, which
survives task 4 harmlessly (`subscriptions_ext` stays used by `.layer`).

**Files:** `server/src/lib.rs` — the now-false comment (`:48-50`), the
`subscriptions_ext` clone (`:51`), the
`.layer(axum::Extension(subscriptions_ext))` (`:126`).

**Steps**

- [ ] **Step 1.** Remove all three. Leave `server/src/context.rs:33`'s Leptos
      context alone — different seam, still needed by
      `web/src/subscriptions/api.rs:23,39` for the write path.
- [ ] **Step 2.** `cargo nextest run -p jaunder --test atompub` → **PASS**.
- [ ] **Step 3.** `cargo xtask check`; commit
      `refactor(server): drop the AtomPub subscription-store extension (#6)`.

---

## Task 8 — correct the records and the remaining stale comments

**Files**

- `docs/adr/0020-content-visibility-and-subscription-model.md:78-80`.
- `docs/adr/0063-domain-value-newtype-convention.md:77-81`.
- `storage/src/posts.rs:1930-1972` and the per-call-site bind comments at
  `:1312, 1343, 1384, 1413, 2312, 2322-2323, 2334, 2345-2346`.

(`web/src/viewer.rs`'s module doc was rewritten in task 4.)

**Steps**

- [ ] **Step 1.** Amend ADR-0020's viewer clause in place with a dated note:
      locality is carried by the type; the `(channel, subscriber_ref)` pair is
      reconstructed in SQL for local viewers. The resolution rule itself is
      unchanged — do not restate or weaken it.
- [ ] **Step 2.** Update ADR-0063's example, which cites the deleted
      `ViewerIdentity::Channel`'s polymorphic `subscriber_ref`. The rule stands;
      point it at the `Local`/`Remote` split as the _applied_ form of that rule.
- [ ] **Step 3.** Fix `resolution_where`'s doc block and every per-call-site
      comment asserting a fixed `$n..$n+4` range — the range is now
      variant-dependent.
- [ ] **Step 4.** `devtool run -- prettier -w` the touched markdown;
      `cargo xtask check`; commit
      `docs(adr): viewer locality is type-level; local channel resolved in SQL (#6)`.

---

## Task 9 — full gate

- [ ] **Step 1.**
      `devtool run --cwd     /home/mdorman/src/jaunder/.claude/worktrees/issue-6-visibility-channel-id     -- cargo xtask validate`
      (Bash background mode — long/cold) → green, including all four
      `{sqlite,postgres}×{chromium,firefox}` e2e combos. **This runs first:**
      `server-fn-coverage regenerate` reads an e2e capture that must already
      exist (`xtask/src/lib.rs:623-629`; a missing capture is a hard error, not
      a no-op).
- [ ] **Step 2.** If the byte-for-byte-gated `docs/coverage/server-fns.json`
      moved, run `cargo xtask server-fn-coverage regenerate` and commit it, then
      re-run `validate`. Expect no movement: `viewer_identity` is not a
      `#[server]` fn and `is_subscribed` survives as one.
- [ ] **Step 3.** Review the whole branch: `git diff wt-base-issue-6..HEAD`.
      Re-read the spec's ACs 1-13 against it before handing to `jaunder-ship`.
