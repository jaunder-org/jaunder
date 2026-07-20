# Typed `#[server]` timestamp boundary (`UtcInstant`) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Replace every bare-`String` RFC3339 timestamp on the web `#[server]`
boundary with a `common::time::UtcInstant` newtype, so timestamps carry their
domain type client→wire→server.

**Architecture:** A serde-transparent `UtcInstant(chrono::DateTime<Utc>)` in
`common` (chrono's `serde` feature enabled there; hand-written `FromStr` for the
client `Field` path). Adopt it at all ~27 boundary sites vertical-by-vertical
(each vertical compiles + its existing tests pass before commit). One free
`Email` adoption. Decision recorded in the already-drafted ADR (promoted at
ship).

**Tech Stack:** Rust, chrono 0.4 (`serde` + default `wasmbind`), serde, leptos
CSR, `thiserror`, `cargo nextest`, `cargo xtask`.

Spec: `docs/superpowers/specs/2026-07-19-issue-91-typed-server-fn-boundary.md`
(the "what/why"; this plan is the "how"). ADR draft:
`docs/adr/drafts/timestamps-cross-boundary-as-utcinstant.md`.

## Global Constraints

- New type lives at `common::time::UtcInstant`, wrapping
  `chrono::DateTime<Utc>`. Wire form is RFC3339 (via chrono's `serde`),
  **compatible with the current String fields**.
- `common/Cargo.toml`: `chrono = { workspace = true, features = ["serde"] }`
  (add the feature; do not otherwise change the dep).
- **Both** the `server` build and the CSR/wasm build must compile at every task
  boundary: `cargo check -p jaunder` (server) **and** `cargo check -p csr`.
- No `.to_rfc3339()` marshalling and no `parse_publish_at` may remain at the web
  boundary when done.
- **Do NOT convert `AudienceSummary.audience_id`/`name`** — sanctioned #475
  `reactive_stores::Patch` carve-out; it stays `i64`/`String`.
- Storage / `common` / `host` internals keep raw `DateTime<Utc>`; `UtcInstant`
  is a boundary/DTO type only.
- Clearing an optional timestamp (`publish_at`) is by **omission → `None`**
  (ADR-0065); an empty wire string is rejected.
- Commit gate: pre-commit hook runs full `cargo xtask check`; run it first
  (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- The ADR is a numberless draft; it is numbered by `cargo xtask adr promote` at
  ship — do not hand-number it.

## Tasks at a glance

- **Task 0** — File a follow-up issue for the deferred `PostSummary`/`Bio`
  newtypes (no existing home).
- **Task 1** — Introduce `UtcInstant` in `common::time`; enable `chrono/serde`.
  (Unit-tested; the one task with genuinely new behavior.)
- **Task 2** — Adopt across the **whole posts vertical** in one atomic change
  (all 7 post DTOs + `publish_at`×2 + 6 cursor params + the shared
  `render/mod.rs::format_post_time` + client). Necessarily one commit —
  `render/mod.rs` is shared across the post DTOs, so the compile unit can't be
  split.
- **Task 3** — Adopt in invites (`InviteInfo`) + the free
  `create_invite.recipient_email → Email`.
- **Task 4** — Adopt in media (`MediaItem.created_at`).
- **Task 5** — Adopt in sessions (`SessionInfo`; only `last_used_at` has a
  display site).
- **Task 6** — e2e: scheduled-publish set **and** clear (omit→None); cursor
  pagination round-trip. Plus `audit-wasm` delta note.

**Key risks/decisions:** (1) chrono's derived serde must stay wire-compatible —
Task 1 pins the exact RFC3339 form with a round-trip + literal-form test; (2)
`publish_at` input flows from the `js_sys::Date` control (kept) →
`UtcInstant::from_str`, and the _clear_ path must survive (Task 6 e2e); (3)
`render/mod.rs::format_post_time` is shared between the web client and the
`server::projector` and takes `&str` today — its signature change to
`&UtcInstant` couples every post DTO, which is why Task 2 is one atomic vertical
(and its date-only fallback branch is deleted, not ported, since a `UtcInstant`
always carries a time); (4) integration-test assertions comparing timestamps to
string literals move to the typed value (compile-driven, per task).

---

### Task 0: File the deferred newtype follow-up

**Files:** none (tracker only).

**Interfaces:** Produces nothing consumed by later tasks; captures separable
scope up front so it isn't lost.

- [x] **Step 1: File the issue** via **jaunder-issues** in
      `jaunder-org/jaunder`, milestone "Domain-value type safety (newtypes)",
      type `task`, label `type-safety`:

  Title:
  `types: PostSummary + Bio newtypes for post excerpt and profile bio (web boundary)`
  Body: the `#[server]` boundary still carries post `summary`/excerpt
  (`create_post`/`update_post` params +
  `*Result`/`PostResponse`/`TimelinePostSummary`/`DraftSummary.summary_label`
  fields) and profile `bio` (`update_profile` param + `ProfileData.bio`) as bare
  `String`. Introduce validated `PostSummary` and `Bio` str-newtypes (ADR-0063
  trailer) and adopt them, mirroring #91's timestamp threading. Surfaced by the
  #91 boundary audit (2026-07-19). Confirm no existing coverage before starting.

- [x] **Step 2:** Recorded: filed as **#545** (`jaunder-org/jaunder`, milestone
      #13, type Task, label `type-safety`, on Backlog project #1). No commit
      (tracker-only).

---

### Task 1: `UtcInstant` newtype + `chrono/serde`

**Files:**

- Create: `common/src/time.rs`
- Modify: `common/src/lib.rs` (add `pub mod time;`, alphabetical — between
  `test_support`/`text` region, before `token`)
- Modify: `common/Cargo.toml:12`
  (`chrono = { workspace = true, features = ["serde"] }`)
- Test: in-file `#[cfg(test)]` in `common/src/time.rs` (the `common` per-type
  convention, e.g. `common/src/slug.rs`)

**Interfaces:**

- Consumes: `chrono::{DateTime, Utc, SecondsFormat}`, `serde`, `thiserror`.
- Produces (relied on by Tasks 2-6):
  - `pub struct UtcInstant(DateTime<Utc>)` — derives
    `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`.
  - `impl UtcInstant { pub fn as_datetime(&self) -> DateTime<Utc>; }`
  - `impl From<DateTime<Utc>> for UtcInstant` (server-side construction from
    storage records).
  - `impl FromStr for UtcInstant { type Err = InvalidInstant; }` (client
    `Field<UtcInstant>` path).
  - `impl Display for UtcInstant` (RFC3339 UTC form).
  - `pub struct InvalidInstant;` —
    `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)] #[error("invalid RFC 3339 timestamp")]`.

- [ ] **Step 1: Write the failing tests** (in-file `#[cfg(test)]`):

```
test "parses an RFC3339 Z string":
    "2026-07-19T10:30:00Z".parse::<UtcInstant>() is Ok
test "parses and canonicalizes an offset to UTC":
    let a = "2026-07-19T15:30:00+05:00".parse::<UtcInstant>().unwrap();
    let b = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
    a == b                                        // same instant
test "FromStr rejects malformed input":
    "not-a-time".parse::<UtcInstant>() == Err(InvalidInstant)
    "2026-13-99".parse::<UtcInstant>() == Err(InvalidInstant)
test "serde round-trips through JSON preserving the instant":
    let x = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
    serde_json::from_str::<UtcInstant>(&serde_json::to_string(&x).unwrap()).unwrap() == x
test "serializes as a bare RFC3339 string (transparent newtype), not an object":
    let json = serde_json::to_string(&"2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap()).unwrap();
    json.starts_with('"') && json.contains("2026-07-19T10:30:00") && !json.contains('{')
test "serde deserialize of an invalid string errors":
    serde_json::from_str::<UtcInstant>("\"not-a-time\"").is_err()
test "as_datetime returns the wrapped UTC instant":
    "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap().as_datetime() == Utc.with_ymd_and_hms(2026,7,19,10,30,0).unwrap()
test "Display emits an RFC3339 UTC string that round-trips":
    let x = UtcInstant::now();
    x.to_string().parse::<UtcInstant>().unwrap() == x
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common time::` — Expected: FAIL (module/type absent).
Then `cargo check -p common` after adding the feature to confirm `chrono/serde`
resolves.

- [ ] **Step 3: Implement against the tests**

Signatures per the Interfaces block. Body determined by the tests: `FromStr` =
`DateTime::parse_from_rfc3339(s).map(|dt| Self(dt.with_timezone(&Utc))).map_err(|_| InvalidInstant)`;
`Display` = `f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::AutoSi, true))`;
serde is derived (transparent newtype over `DateTime`'s chrono/serde). Every
branch (parse-ok, offset-canonicalize, parse-err, round-trip, transparent-form,
invalid-deserialize) is pinned above.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common time::` — Expected: PASS. Then **both**
builds: `cargo check -p common` and `cargo check -p csr` (proves `UtcInstant` +
`chrono/serde` compile for wasm). Expected: PASS.

- [ ] **Step 5: Commit** (`cargo xtask check` first)

```bash
git add common/src/time.rs common/src/lib.rs common/Cargo.toml
git commit -m "feat(common): UtcInstant timestamp newtype for the web boundary"
```

---

### Task 2: Adopt `UtcInstant` across the whole posts vertical (atomic)

This is deliberately one task/commit: `web/src/render/mod.rs::format_post_time`
(`&str` today) is shared between the web client and `server::projector` and is
called with fields from **multiple** post DTOs, so its signature change to
`&UtcInstant` couples every post DTO into one compile unit — it cannot be split
without a non-compiling intermediate.

**Files:**

- Modify: `web/src/posts/listing.rs` (`TimelinePage.next_cursor_created_at`,
  `TimelinePostSummary.created_at`/`published_at`; the 5 `list_*` cursor params;
  server production `:79,:224`)
- Modify: `web/src/posts/mod.rs`
  (`DraftSummary.created_at`/`updated_at`/`scheduled_at`; `CreatePostResult`,
  `UpdatePostResult`, `PublishPostResult`, `PostResponse` timestamp fields;
  `create_post`/`update_post` `publish_at: Option<String>` →
  `Option<UtcInstant>`; `list_drafts.cursor_created_at`; **delete
  `parse_publish_at` `:221`**; server production
  `:296-297,:474,:556,:562-563,:626`)
- Modify: `web/src/posts/server.rs` (production `:33-34,:76-78`)
- Modify: `web/src/render/mod.rs` (`format_post_time` signature `&str` →
  `&UtcInstant`; **delete the date-only fallback branch** — a `UtcInstant`
  always carries a time; rewrite the in-file `sample_post()`/`sample_summary()`
  fixtures `:760-761,:781-782` from `"…".into()` to
  `"…".parse::<UtcInstant>().unwrap()`; rewrite the 4 `format_post_time_*` tests
  and **delete** `format_post_time_handles_date_only_input`)
- Modify: `web/src/pages/timeline.rs`, `web/src/pages/posts.rs`,
  `web/src/pages/ui.rs` (cursor signals `RwSignal<Option<String>>` →
  `RwSignal<Option<UtcInstant>>`; draft-list/timeline/editor display;
  `scheduled_badge` `:1018`; `publish_at` input construction; message paths
  `posts.rs:56,:225-229,:652,:708`, `ui.rs:760,767`)
- Test: existing `server/tests` post/timeline/drafts integration tests (update
  assertions) + the rewritten `render/mod.rs` in-file tests.

**Interfaces:**

- Consumes: `common::time::UtcInstant` (Task 1).
- Produces: all 7 post DTOs' timestamps + `publish_at` + cursors typed as
  `UtcInstant`/`Option<UtcInstant>`;
  `format_post_time(ts: &UtcInstant) -> String` (Tasks 3-5 don't use it — they
  render directly). `parse_publish_at` deleted (arg-decode validates
  `publish_at`).

- [ ] **Step 1: Change `format_post_time` + its fixtures/tests.** Signature
      `&str` → `&UtcInstant`; format via `ts.as_datetime()` + chrono. Delete the
      no-`T` date-only fallback branch (unreachable once the param is a full
      instant) and its `format_post_time_handles_date_only_input` test; rewrite
      the other 3 `format_post_time_*` tests to build `UtcInstant`; fix
      `sample_post()`/`sample_summary()` fixture fields to
      `.parse::<UtcInstant>().unwrap()`.

- [ ] **Step 2: Change the DTO + param types + server production.** Replace
      every `String`/`Option<String>` timestamp field/param across
      `posts/mod.rs`, `posts/listing.rs`, `posts/server.rs` with
      `UtcInstant`/`Option<UtcInstant>`. Production `dt.to_rfc3339()` →
      `dt.into()` (`UtcInstant::from`). Delete `parse_publish_at`; the fn now
      gets an already-validated `Option<UtcInstant>` — use
      `.map(|t| t.as_datetime())` where storage wants `DateTime<Utc>`.

- [ ] **Step 3: Thread the client.** Cursor signals → `Option<UtcInstant>`.
      Display sites format via `.as_datetime()`/`format_post_time`/`Display`.
      `publish_at` submit sites (`ui.rs:476,488,592`, `posts.rs:717`): keep
      `local_datetime_to_utc_rfc3339` (browser local→UTC) and parse its output —
      `local_datetime_to_utc_rfc3339(raw).and_then(|s| s.parse::<UtcInstant>().ok())`;
      the **clear** path yields `None`.

- [ ] **Step 4: Update assertions.** Fix `server/tests` post/timeline/drafts
      assertions comparing a timestamp to a `String` literal →
      `"…".parse::<UtcInstant>().unwrap()` (or `.as_datetime()`; the
      `web_posts.rs` cursor-query-string assertions still expect the RFC3339
      wire text).

- [ ] **Step 5: Compile both targets + run tests.**

Run: `cargo check -p jaunder` && `cargo check -p csr` (Expected: PASS).
`cargo nextest run -p jaunder --test integration posts` and `-p web render`
(Expected: PASS, incl. scheduled-publish).
`rg 'parse_publish_at|to_rfc3339' web/src/posts web/src/render/mod.rs` —
Expected: no matches.

- [ ] **Step 6: Commit** (`cargo xtask check` first)

```bash
git add web/src/posts web/src/render/mod.rs web/src/pages/posts.rs web/src/pages/ui.rs web/src/pages/timeline.rs server/tests
git commit -m "refactor(web): type all post-vertical timestamps + publish_at as UtcInstant"
```

---

### Task 3: Adopt in invites + free `Email` adoption

**Files:**

- Modify: `web/src/invites/mod.rs`
  (`InviteInfo.created_at`/`expires_at`/`used_at` →
  `UtcInstant`/`Option<UtcInstant>`; production `:103-105`;
  **`create_invite.recipient_email: String` → `Email`** `:37`)
- Modify: `web/src/pages/invites.rs` (display `:86-88`; the invite-create form
  call site now passes/validates `Email` — use `Field::<Email>` per ADR-0065 if
  the form validates it, else construct at submit)
- Test: existing `server/tests` invites tests (update assertions).

**Interfaces:**

- Consumes: `UtcInstant` (Task 1), `common::email::Email` (exists).
- Produces: `InviteInfo` typed; `create_invite` takes `Email`.

- [ ] **Step 1: Change the types** (`InviteInfo` fields; `recipient_email`).
      Production `.to_rfc3339()` → `.into()`. If the create-invite form
      currently sends a raw email string, pre-validate with `Field::<Email>` /
      `ValidatedInput<Email>` (ADR-0065) so decode can't surface a generic
      error; otherwise the value is already an `Email` at the call site.

- [ ] **Step 2: Update assertions** in `server/tests` invites tests
      (compile-driven), incl. any that asserted `recipient_email` as a `String`.

- [ ] **Step 3: Compile both targets + run tests.**

Run: `cargo check -p jaunder` && `cargo check -p csr` (Expected: PASS).
`cargo nextest run -p jaunder --test integration invites` (Expected: PASS).
`rg 'to_rfc3339' web/src/invites/mod.rs` — Expected: no matches.

- [ ] **Step 4: Commit** (`cargo xtask check` first)

```bash
git add web/src/invites/mod.rs web/src/pages/invites.rs server/tests
git commit -m "refactor(web): type invite timestamps as UtcInstant + recipient_email as Email"
```

---

### Task 4: Adopt in media

**Files:**

- Modify: `web/src/media/mod.rs` (`MediaItem.created_at` → `UtcInstant`;
  production `:83`)
- Modify: `web/src/pages/media.rs` (display `:166`)
- Test: existing `server/tests` media tests (update assertions).

**Interfaces:** Consumes `UtcInstant` (Task 1). Produces
`MediaItem.created_at: UtcInstant`.

- [ ] **Step 1: Change the type**, production `.to_rfc3339()` → `.into()`,
      display via `.as_datetime()`/`Display`.
- [ ] **Step 2: Update assertions** in `server/tests` media tests
      (compile-driven).
- [ ] **Step 3: Compile both targets + run tests.** `cargo check -p jaunder` &&
      `cargo check -p csr`;
      `cargo nextest run -p jaunder --test integration media`.
      `rg 'to_rfc3339' web/src/media/mod.rs` — no matches. Expected: PASS.
- [ ] **Step 4: Commit** (`cargo xtask check` first)

```bash
git add web/src/media/mod.rs web/src/pages/media.rs server/tests
git commit -m "refactor(web): type media created_at as UtcInstant"
```

---

### Task 5: Adopt in sessions

**Files:**

- Modify: `web/src/sessions/mod.rs` (`SessionInfo.created_at`/`last_used_at` →
  `UtcInstant`; production `:37-38`)
- Modify: `web/src/pages/sessions.rs` (the one display site —
  `{s.last_used_at.clone()}` `:70`, now `.to_string()`/`Display`; `created_at`
  is typed in the DTO but not rendered)
- Test: existing `server/tests` sessions tests (update assertions).

**Interfaces:** Consumes `UtcInstant` (Task 1). Produces `SessionInfo`
timestamps typed.

- [ ] **Step 1: Change the types**, production `.to_rfc3339()` → `.into()`,
      display via `.as_datetime()`/`Display`.
- [ ] **Step 2: Update assertions** in `server/tests` sessions tests
      (compile-driven).
- [ ] **Step 3: Compile both targets + run tests.** `cargo check -p jaunder` &&
      `cargo check -p csr`;
      `cargo nextest run -p jaunder --test integration sessions`. Then the
      definitive sweep: `rg 'to_rfc3339' web/src` — Expected: **no matches
      anywhere in `web/src`** (criterion 2). Expected: PASS.
- [ ] **Step 4: Commit** (`cargo xtask check` first)

```bash
git add web/src/sessions/mod.rs web/src/pages/sessions.rs server/tests
git commit -m "refactor(web): type session timestamps as UtcInstant"
```

---

### Task 6: e2e (schedule + clear, pagination) + wasm-bundle note

**Files:**

- Modify/Create: the e2e spec covering scheduled publishing (`end2end/` — extend
  the #70 scheduled-publish spec) and a pagination check.
- Test: Playwright e2e.

**Interfaces:** Consumes the full typed boundary (Tasks 2-6). Produces the
behavioral proof for spec criteria 5-6.

- [ ] **Step 1: e2e — schedule a post.** Drive the datetime control, submit
      `create_post` with a future `publish_at`, assert the draft shows the
      scheduled badge and the post is not yet public. (Reuses/extends the #70
      flow; confirms `Option<UtcInstant>` sends and decodes.)
- [ ] **Step 2: e2e — clear the schedule.** Edit the scheduled post, clear the
      datetime field, submit `update_post`; assert the schedule is removed (omit
      → `None` path per ADR-0065), i.e. the badge is gone / post publishes
      immediately per the existing semantics.
- [ ] **Step 3: e2e — pagination round-trip.** On a timeline with > 1 page, load
      the first page and page again; assert the second page loads (the typed
      `UtcInstant` cursor round-trips), no duplicates/gaps.
- [ ] **Step 4: Run e2e.** `cargo xtask e2e sqlite chromium` (Expected: PASS).
      (Full matrix runs in `validate`.)
- [ ] **Step 5: wasm-bundle note.** `cargo xtask audit-wasm`; record the
      raw/gzip delta vs. `main` in the PR description (expected ≈ 0 — chrono
      already in the bundle; only chrono/serde impls added).
- [ ] **Step 6: Commit** (`cargo xtask check` first)

```bash
git add end2end
git commit -m "test(e2e): typed scheduled-publish set/clear + cursor pagination"
```

---

## Final gate (before ship)

- [ ] `cargo xtask validate` green (static + coverage + full e2e matrix).
- [ ] `rg 'to_rfc3339' web/src` → no matches;
      `rg 'Option<String>|: String' web/src` over the enumerated timestamp
      fields → none remain typed as strings.
- [ ] ADR promoted (`cargo xtask adr promote`) in **jaunder-ship**, not here.
