# PostSummary + Bio newtypes — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Introduce validated `PostSummary` (≤500) and `Bio` (≤1000) string
newtypes in `common` and thread each through the full vertical (storage → render
→ feed → web → form), so summary/bio are parsed once at the boundary and held
typed everywhere inward.

**Architecture:** Two `#[derive(StrNewtype)]` newtypes with hand-written
validating `FromStr` (the `DisplayName` template). The
**storage↔web↔feed↔form core** for each value is genuinely atomic: those
`summary`/`bio` fields are connected through move sites (`post_response`, feed
`regenerate`, the create/update path, the form's prefill+submit), so a proper
subset there cannot compile — the core lands in one green commit per value.
(`common::render::DerivedPostMetadata.summary_label` is a separable compilation
island — built and read only within `render.rs` — so it could stand alone, but
is folded into Task 3 for cohesion since it is the same `truncated`-door
change.) Spec:
[`2026-07-20-issue-545-postsummary-bio-newtypes.md`](../specs/2026-07-20-issue-545-postsummary-bio-newtypes.md).

**Tech Stack:** Rust, `macros::StrNewtype` (ADR-0063/0071), `thiserror`, Leptos
`Field<T>` client validation (ADR-0065), `sqlx` (SQLite + Postgres),
`cargo nextest`, Playwright e2e.

## Review header

**Scope (in):** `PostSummary`/`Bio` newtypes in `common`; threading through
`storage` (records, inputs, `fallback_summary_label`, sqlx binds),
`common::render` (`DerivedPostMetadata.summary_label`), `common::feed`
(`FeedItem.summary` + renderers), `web` (post/profile DTOs, `#[server]` args,
seam builders), and the compose/profile forms (`Field<T>`); a one-paragraph
ADR-0063 addendum for the `truncated` door; e2e for the clear paths and
disable-until-valid UX.

**Scope (out):** timestamps (#91); any data backfill for over-cap legacy rows
(accepted fail-closed); a `Field`/`ValidatedInput` macro. No DB migration
(columns stay `TEXT`).

**Tasks:**

1. `PostSummary` newtype in `common` (+ `truncated` door, + ADR-0063 addendum).
   Green isolated.
2. `Bio` newtype in `common`. Green isolated.
3. Thread `PostSummary` through the vertical (storage + render + feed + web +
   compose form). Atomic.
4. Thread `Bio` through the vertical (storage + web profile + profile form).
   Atomic.
5. e2e: clear-bio, clear-summary, over-cap disable-submit.
6. Full-gate validation + acceptance sweep (`cargo xtask validate`, grep
   criteria).

**Key risks/decisions:** (a) per-value atomic threading to stay green (above);
(b) `truncated` is a length-only trusted door — non-emptiness is
caller-guaranteed + `debug_assert!` (spec Risks); (c) validating `Decode` ⇒
over-cap legacy rows fail-closed, mirroring `DisplayName`
(`storage/src/users.rs:637`); accepted.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity** (`CONTRIBUTING.md`): storage tests are
  `#[case]`-parametrized over `Backend::{Sqlite,Postgres}`, bind the whole
  `TestEnv` (ADR-0053), and use `common::test_support::parse_*` helpers — never
  inline `.parse().unwrap()`.
- **Coverage policy / verify ladder** per `CONTRIBUTING.md`; the pre-commit hook
  runs `cargo xtask check` (fmt + clippy + Nix coverage/tests) — run it clean
  before each commit (**jaunder-commit**), then `git status --porcelain` (fmt
  auto-fix may leave the tree dirty).
- **No `#[allow]`/`#[expect]`** to silence lints except the sanctioned
  `test_support` `expect_used` case; no filesystem-wide search; do not commit
  without approval already granted for this cycle.
- New crate deps: none (reuses `macros`, `thiserror`).

---

### Task 1: `PostSummary` newtype in `common`

**Files:**

- Create: `common/src/post_summary.rs`
- Modify: `common/src/lib.rs` (register `pub mod post_summary;`)
- Modify: `docs/adr/0063-domain-value-newtype-convention.md` (truncating-door
  addendum)

> **Note (stateless coverage gate, ADR-0050):** `parse_post_summary` is **not**
> added here — a `test_support` helper is coverage-measured, and with no
> consumer yet it would be uncovered at this commit. It lands in **Task 3**
> alongside its first caller. Same for `parse_bio` (Task 4).

**Interfaces:**

- Produces:
  `common::post_summary::{PostSummary, InvalidPostSummary, MAX_POST_SUMMARY_CHARS}`;
  `PostSummary: FromStr<Err = InvalidPostSummary>` (validating), full
  `StrNewtype` trailer (`Display`, `AsRef`/`Borrow`/`Deref<str>`,
  `TryFrom<String>`/`From<Self> for String`, `PartialEq<str>`/`<&str>`,
  validating serde bridge, ADR-0071 sqlx bridge);
  `PostSummary::truncated(&str) -> PostSummary` (infallible, length-capped);
  const `MAX_POST_SUMMARY_CHARS = 500`;
  `common::test_support::parse_post_summary(&str) -> PostSummary`.

- [ ] **Step 1: Write the failing tests** (`common/src/post_summary.rs`
      `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_trims_preserving_inner_and_case() {
        assert_eq!("  Hello  World  ".parse::<PostSummary>().unwrap(), "Hello  World");
        // inner newlines preserved
        assert_eq!("line1\nline2".parse::<PostSummary>().unwrap(), "line1\nline2");
        assert_eq!("Пример".parse::<PostSummary>().unwrap(), "Пример");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!("".parse::<PostSummary>().is_err());
        assert!("   \t\n".parse::<PostSummary>().is_err());
    }

    #[test]
    fn enforces_length_cap_on_scalars_post_trim() {
        let max: String = "a".repeat(MAX_POST_SUMMARY_CHARS);
        assert!(max.parse::<PostSummary>().is_ok());
        let over: String = "a".repeat(MAX_POST_SUMMARY_CHARS + 1);
        assert!(over.parse::<PostSummary>().is_err());
        // surrounding whitespace doesn't push an at-cap value over
        let padded = format!("  {}  ", "a".repeat(MAX_POST_SUMMARY_CHARS));
        assert!(padded.parse::<PostSummary>().is_ok());
    }

    #[test]
    fn serde_serializes_plain_string_and_validates_on_deserialize() {
        let s: PostSummary = "Blurb".parse().unwrap();
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"Blurb\"");
        assert_eq!(serde_json::from_str::<PostSummary>("\"Blurb\"").unwrap(), s);
        assert!(serde_json::from_str::<PostSummary>("\"\"").is_err());
        let over = format!("\"{}\"", "a".repeat(MAX_POST_SUMMARY_CHARS + 1));
        assert!(serde_json::from_str::<PostSummary>(&over).is_err());
    }

    #[test]
    fn truncated_trims_and_caps_at_char_boundary() {
        // under cap: unchanged (trimmed)
        assert_eq!(PostSummary::truncated("  hi  "), "hi");
        // over cap: truncated to exactly MAX scalars, no panic on multibyte
        let over: String = "é".repeat(MAX_POST_SUMMARY_CHARS + 50);
        let t = PostSummary::truncated(&over);
        assert_eq!(t.chars().count(), MAX_POST_SUMMARY_CHARS);
    }

    #[test]
    #[should_panic(expected = "non-empty")]
    fn truncated_debug_asserts_non_empty() {
        // documents the caller-trusted precondition (debug builds)
        let _ = PostSummary::truncated("   ");
    }
}
```

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p common post_summary`
      Expected: FAIL — `PostSummary` undefined.

- [ ] **Step 3: Implement `common/src/post_summary.rs`** to this exact contract:

```rust
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// Maximum post-summary length, in Unicode scalar values.
pub const MAX_POST_SUMMARY_CHARS: usize = 500;

/// A validated post summary/excerpt: outer whitespace trimmed, non-empty, at most
/// [`MAX_POST_SUMMARY_CHARS`] scalars; inner whitespace/newlines and case preserved.
///
/// The **public** doors ([`FromStr`], the serde/sqlx bridges) enforce the full invariant
/// (non-empty AND ≤ cap). [`PostSummary::truncated`] is an internal *trusted* door that
/// guarantees only the length cap (see its docs). The ADR-0063 string trailer is
/// generated by `#[derive(StrNewtype)]`. No `Hash` — never a map/set key.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PostSummary(String);

/// Error returned when a string cannot be parsed as a [`PostSummary`].
#[derive(Debug, Error)]
#[error("post summary must be non-empty and at most {MAX_POST_SUMMARY_CHARS} characters")]
pub struct InvalidPostSummary;

impl FromStr for PostSummary {
    type Err = InvalidPostSummary;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_POST_SUMMARY_CHARS {
            return Err(InvalidPostSummary);
        }
        Ok(PostSummary(trimmed.to_owned()))
    }
}

impl PostSummary {
    /// Build a `PostSummary` from an internally-derived, **non-empty** label (a post's
    /// first body line, title, or slug), truncating to [`MAX_POST_SUMMARY_CHARS`] scalars.
    ///
    /// Infallible length-validated **trusted** door — the string analog of
    /// `NumNewtype::clamped` and the `RenderedHtml::from_trusted` model. It guarantees the
    /// length cap but **not** non-emptiness (that half is enforced only by [`FromStr`]/serde);
    /// callers must pass non-empty input, which the `debug_assert!` pins in test/debug.
    /// Sole callers: `storage::PostRecord::fallback_summary_label` and
    /// `common::render::derive_post_metadata`.
    #[must_use]
    pub fn truncated(s: &str) -> Self {
        let trimmed = s.trim();
        debug_assert!(!trimmed.is_empty(), "PostSummary::truncated requires non-empty input");
        PostSummary(trimmed.chars().take(MAX_POST_SUMMARY_CHARS).collect())
    }
}
```

Register in `common/src/lib.rs` (`pub mod post_summary;`, alphabetical among
siblings). Add to `common/src/test_support.rs` (import `PostSummary`, mirror
`parse_display_name`):

```rust
/// Parse `s` into a valid [`PostSummary`] for tests — the single test-construction door.
///
/// # Panics
/// Panics if `s` is empty/whitespace-only or longer than the length bound.
#[must_use]
pub fn parse_post_summary(s: &str) -> PostSummary {
    s.parse().expect("valid test post summary")
}
```

- [ ] **Step 4: ADR-0063 addendum.** In
      `docs/adr/0063-domain-value-newtype-convention.md`, add one sentence to §2
      (near the numeric `clamped` paragraph) documenting the **string**
      truncating trusted door: a hand-written `truncated(&str) -> Self` that
      trims + truncates to the cap — the string analog of `clamped`, a validated
      (length-only) door that trusts the caller for the non-length half of the
      invariant; first user `PostSummary` (#545). Run
      `prettier -w docs/adr/0063-domain-value-newtype-convention.md` before
      staging (pre-commit prose parity).

- [ ] **Step 5: Run, verify pass** — `cargo nextest run -p common post_summary`
      → PASS. Also `cargo nextest run -p common test_support` compiles the
      helper.

- [ ] **Step 6: Commit** (run `cargo xtask check` clean first, then
      `git status --porcelain`)

```bash
git add common/src/post_summary.rs common/src/lib.rs common/src/test_support.rs \
        docs/adr/0063-domain-value-newtype-convention.md
git commit -m "feat(common): add validated PostSummary newtype (#545)"
```

---

### Task 2: `Bio` newtype in `common`

**Files:**

- Create: `common/src/bio.rs`
- Modify: `common/src/lib.rs` (register `pub mod bio;`)
- Modify: `common/src/test_support.rs` (add `parse_bio`)

**Interfaces:**

- Produces: `common::bio::{Bio, InvalidBio, MAX_BIO_CHARS}`;
  `Bio: FromStr<Err = InvalidBio>` (validating; trim, non-empty, ≤
  `MAX_BIO_CHARS = 1000`) with the full `StrNewtype` trailer + serde + sqlx
  bridges; `common::test_support::parse_bio(&str) -> Bio`. No `truncated` door
  (bio is always user input, never derived).

- [ ] **Step 1: Write the failing tests** (`common/src/bio.rs` `#[cfg(test)]`) —
      mirror the `DisplayName` tests:
      `parses_and_trims_preserving_inner_and_case` (incl. an inner newline + a
      Unicode string), `rejects_empty_and_whitespace_only`,
      `enforces_length_cap` (at `MAX_BIO_CHARS` ok, `+1` err, padded-at-cap ok),
      `serde_serializes_plain_string_and_validates_on_deserialize` (round-trip +
      reject `""`
  - reject over-cap).

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p common bio` → FAIL
      (undefined).

- [ ] **Step 3: Implement `common/src/bio.rs`** to this exact contract
      (DisplayName shape, cap 1000, error message "biography must be non-empty
      and at most {MAX_BIO_CHARS} characters"):

```rust
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// Maximum profile-biography length, in Unicode scalar values.
pub const MAX_BIO_CHARS: usize = 1000;

/// A validated user biography: outer whitespace trimmed, non-empty, at most
/// [`MAX_BIO_CHARS`] scalars; inner whitespace/newlines and case preserved. Absence of a
/// bio is modeled by `Option<Bio>` at the boundary, so `FromStr` rejects the empty string
/// (an empty wire value is rejected; clearing goes through omission → `None`). The
/// ADR-0063 string trailer is generated by `#[derive(StrNewtype)]`. No `Hash`.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct Bio(String);

/// Error returned when a string cannot be parsed as a [`Bio`].
#[derive(Debug, Error)]
#[error("biography must be non-empty and at most {MAX_BIO_CHARS} characters")]
pub struct InvalidBio;

impl FromStr for Bio {
    type Err = InvalidBio;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_BIO_CHARS {
            return Err(InvalidBio);
        }
        Ok(Bio(trimmed.to_owned()))
    }
}
```

Register `pub mod bio;` in `common/src/lib.rs`; add `parse_bio` to
`test_support.rs` (mirror `parse_post_summary`, panic message "valid test bio").

- [ ] **Step 4: Run, verify pass** — `cargo nextest run -p common bio` → PASS.

- [ ] **Step 5: Commit** (`cargo xtask check` clean, `git status --porcelain`)

```bash
git add common/src/bio.rs common/src/lib.rs common/src/test_support.rs
git commit -m "feat(common): add validated Bio newtype (#545)"
```

---

### Task 3: Thread `PostSummary` through the vertical

One atomic commit — the tree must compile green, so every `summary`-typed
surface flips together. Work outward from the data layer; drive by updating
tests first where a test pins new behavior, otherwise the change is a mechanical
type flip the compiler verifies.

**Files (flip `summary`/`summary_label` →
`PostSummary`/`Option<PostSummary>`):**

- Modify: `storage/src/posts.rs` — `PostRecord.summary` (:71),
  `CreatePostInput`/ `UpdatePostInput` summary (:206, :233),
  `fallback_summary_label` (:93 → returns `PostSummary` via
  `PostSummary::truncated`), the `INSERT` bind (:1902 → bind
  `Option<&PostSummary>` directly, no `.as_deref()`), and every `SELECT` decode
  of the `summary` column (→ `Option<PostSummary>` via the sqlx bridge).
- Modify: `storage/src/post_service.rs` — summary fields on the service arg
  structs (:42, :143, :279, :443) → `Option<PostSummary>`.
- Modify: `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs` —
  `summary` column decode rows.
- Modify: `common/src/render.rs` — `DerivedPostMetadata.summary_label` (:140) →
  `PostSummary`; each `derive_post_metadata` assembly branch builds it via
  `PostSummary::truncated(&label)`; `fallback_label` helper (:188) stays
  `Option<String>`; `slug_seed` stays `String`.
- Modify: `common/src/feed/metadata.rs` — `FeedItem.summary` (:24) →
  `Option<PostSummary>`.
- Modify: `server/src/feed/regenerate.rs` — `summary: p.summary.clone()` (:151)
  now moves the typed value; no conversion.
- Modify: `common/src/feed/atom.rs` (:56) and `json.rs` (:20) — read the summary
  out via `Deref`/`Display`/`Serialize` at the external-crate boundary (ADR-0063
  §5 carve-out, mirroring the existing `title` handling: e.g.
  `Text::plain(s.to_string())`). `rss.rs` does not emit summary (only
  `summary: None` test data) — no production site to change, but its
  `FeedItem { .. }` test constructors compile against the new type.
- Modify: `web/src/posts/mod.rs` — DTO fields `CreatePostResult.summary` (:64),
  `UpdatePostResult.summary` (:75), `PostResponse.summary` (:214) →
  `Option<PostSummary>`; `DraftSummary.summary_label` (:162) → `PostSummary`;
  `#[server]` args `create_post` (:254) / `update_post` (:409) summary →
  `Option<PostSummary>`; **remove** the two
  `summary.and_then(common::text::non_empty_owned)` normalization sites (FromStr
  already trims/rejects-empty).
- Modify: `web/src/posts/listing.rs` — `TimelinePostSummary.summary` (:40) →
  `Option<PostSummary>`.
- Modify: `web/src/posts/server.rs` — `post_response` / `timeline_post_summary`
  move the typed `summary` directly (field types change only); update the
  `post_response_carries_summary` test (`summary: Some("the summary".into())` →
  `Some(parse_post_summary("the summary"))`, assert equals same).
- Modify: `web/src/pages/posts.rs` — the compose/edit summary control
  (`RwSignal<String>` at :637, textarea `#edit-summary` :795, submit
  `common::text::non_empty_owned(summary.get())` at :722, prefill at :706) → a
  parent-owned `Field::<PostSummary>::optional()` /
  `optional_prefilled(existing)`, direct-bound to the textarea (ADR-0065
  direct-bind variant, mirroring `Field::<DisplayName>::optional()` in
  `web/src/pages/profile.rs:18`), submitting `summary: field.parsed()`
  (`Option<PostSummary>`); touched-gated inline error.

**Interfaces:**

- Consumes: `PostSummary`, `parse_post_summary` (Task 1).
- Produces: all the above surfaces typed;
  `PostRecord.summary: Option<PostSummary>`,
  `FeedItem.summary: Option<PostSummary>`, the web DTOs/args typed — Task 5
  (e2e) relies on the wired form.

- [x] **Step 1: Storage tests first** (`storage/src/posts.rs` `#[cfg(test)]`).
      Update `create_post_persists_summary` to build via
      `parse_post_summary("the summary")` and assert
      `post.summary == Some(parse_post_summary("the summary"))`. Update
      `fallback_summary_label_*` tests to expect `PostSummary`
      (`assert_eq!(post.fallback_summary_label(), parse_post_summary("..."))`).
      Add a **fail-closed** dual-backend test mirroring `users.rs:637`:

```rust
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn reading_post_with_overlong_summary_in_db_errors(#[case] backend: Backend) {
    let env = TestEnv::new(backend).await;
    // seed a valid post, then force an over-cap summary via raw SQL (unconstructible
    // via the newtype) — see server crate seed recipe / env.base.pool().execute(...)
    let overlong = "a".repeat(common::post_summary::MAX_POST_SUMMARY_CHARS + 1);
    // UPDATE posts SET summary = <overlong> WHERE ...
    // then get_post(...) must return Err (decode fails-closed), not Ok.
    // assert get result is_err()
}
```

- [ ] **Step 2: Run storage tests, verify fail** —
      `cargo nextest run -p storage posts::` → FAIL (type mismatch / new test
      unimplemented).

- [x] **Step 3: Implement the flip** across the Files list above. The changes
      are mechanical type substitutions the compiler pins, except:
      `fallback_summary_label`/ `derive_post_metadata` construct via
      `PostSummary::truncated`; the feed renderers deref out; the compose form
      adopts `Field<PostSummary>`. No `.parse().expect()` at any web/feed seam
      (grep to confirm). Iterate until
      `cargo check --all-features --all-targets -p common -p storage -p server -p web`
      is clean.

- [x] **Step 4: Web host test** (`web/src/posts/...` `#[cfg(test)]`) — add a
      `field_error::<PostSummary>` host test (over-cap input → `Some(msg)`,
      empty → `None` under `Field::optional`, valid → `None`), mirroring the
      existing `field_error` tests for `Username`/`DisplayName`.

- [x] **Step 5: Run, verify pass** —
      `cargo nextest run -p common -p storage -p server -p web`
      (post/feed/render tests) → PASS. Run
      `cargo check --all-features --all-targets` (server-gated web code, per
      `project_default_check_skips_server_gated_web`).

- [ ] **Step 6: Commit** (`cargo xtask check` clean, `git status --porcelain`)

```bash
git add common/ storage/ server/ web/
git commit -m "refactor(posts): thread PostSummary through storage, feed, and web (#545)"
```

---

### Task 4: Thread `Bio` through the vertical

One atomic commit (profile surface is smaller; input and output both live in
`profile/mod.rs` + `users.rs`).

**Files (flip `bio` → `Bio`/`Option<Bio>`):**

- Modify: `storage/src/users.rs` — `User.bio` (:32) → `Option<Bio>`;
  `ProfileUpdate.bio` (:118) → `Option<&'a Bio>` (mirror
  `display_name: Option<&DisplayName>`); the `UPDATE ... SET bio = $2` bind
  (:423, `.bind(update.bio)` via the sqlx bridge) and the `SELECT` bio-column
  decode (`users.rs:334`, within the `SELECT` at :317) → `Bio`.
- Modify: `web/src/profile/mod.rs` — `ProfileData.bio` (:27) → `Option<Bio>`;
  `update_profile` arg (:58) → `bio: Option<Bio>`; body passes `bio.as_ref()`
  into `ProfileUpdate` and **drops** the `common::text::non_empty(&bio)` shim
  (:62); `get_profile` moves `user.bio` directly into `ProfileData.bio`.
- Modify: `web/src/pages/profile.rs` — the bio control
  (`RwSignal::new(String::new())` at :19, textarea `name="bio"` :74, submit
  `bio: bio.get()` :41) → a parent-owned `Field::<Bio>::optional()` /
  `optional_prefilled(existing_bio)`, direct-bound to the textarea (mirror the
  `Field::<DisplayName>::optional()` control already at :18), submitting
  `bio: field.parsed()` (`Option<Bio>`).

**Interfaces:**

- Consumes: `Bio`, `parse_bio` (Task 2).
- Produces: `User.bio: Option<Bio>`, `ProfileUpdate.bio: Option<&Bio>`,
  `ProfileData.bio: Option<Bio>`, `update_profile(bio: Option<Bio>)`.

- [ ] **Step 1: Storage tests first** (`storage/src/users.rs` `#[cfg(test)]`).
      Update the profile round-trip tests to set/read `bio` via
      `parse_bio(...)`; add a dual-backend set→clear test
      (`ProfileUpdate { bio: Some(&parse_bio("hi")), .. }` then `bio: None`
      clears) asserting the round-tripped `User.bio`; add a fail-closed
      `reading_user_with_overlong_bio_in_db_errors` mirroring the `display_name`
      overlong test at `users.rs:637` (raw-SQL over-cap bio → authenticate/get
      returns internal error).

- [ ] **Step 2: Run, verify fail** — `cargo nextest run -p storage users::` →
      FAIL.

- [ ] **Step 3: Implement the flip** across the Files list. Mechanical except
      the form's `Field<Bio>` adoption. Confirm no `.parse().expect()` at the
      profile seam.

- [ ] **Step 4: Web host test** — add a `field_error::<Bio>` host test (over-cap
      → error, empty → `None` under optional, valid → `None`).

- [ ] **Step 5: Run, verify pass** —
      `cargo nextest run -p storage -p web users profile` → PASS;
      `cargo check --all-features --all-targets` clean.

- [ ] **Step 6: Commit** (`cargo xtask check` clean, `git status --porcelain`)

```bash
git add storage/ web/
git commit -m "refactor(profile): thread Bio through storage and the web boundary (#545)"
```

---

### Task 5: e2e — clear paths + disable-until-valid

**Files:**

- Modify/Create: the post e2e spec (`end2end/tests/posts.spec.ts` or the
  existing compose/edit spec) and the profile e2e spec
  (`end2end/tests/profile.spec.ts`) — match the repo's existing spec
  naming/fixtures.

**Interfaces:**

- Consumes: the wired forms from Tasks 3–4.

- [ ] **Step 1: Write the e2e scenarios** (Playwright, following existing specs'
      fixtures / per-test identity, ADR-0039):
  - **Bio clear:** log in → set a bio → save → reload → bio present; then
    **clear** the bio (empty the textarea) → save → reload → bio empty/absent
    (verifies the `None`-omission clear path in the browser, per
    `project_adr0065_optional_field_clearing`).
  - **Summary clear:** create a post with a summary → edit → clear the summary →
    save → reopen → summary empty.
  - **Disable-until-valid:** type an over-cap summary (and over-cap bio) → the
    inline validation error shows and the submit/save button is `disabled`;
    shorten to valid → button enabled.

- [ ] **Step 2: Run against one combo, verify (expect PASS after 3–4 land)** —
      `cargo xtask e2e sqlite chromium` (worktree-aware). Investigate any local
      stale-server flake (`ss :3000`) per
      `project_host_e2e_stale_server_false_negative`.

- [ ] **Step 3: Commit** (`cargo xtask check` clean)

```bash
git add end2end/
git commit -m "test(e2e): cover PostSummary/Bio clear paths and disable-until-valid (#545)"
```

---

### Task 6: Full-gate validation + acceptance sweep

**Files:** none (verification only; fold any fmt/clippy fixes into the relevant
task's commit via `--fixup` per `feedback_fixup_clean_history` rather than a
churn commit).

- [ ] **Step 1: Acceptance grep** (spec criteria 3–4) — confirm no production
      `summary:`, `summary_label:`, or `bio:` field is typed
      `String`/`Option<String>` on the enumerated surfaces, and no
      `.parse().expect()`/`.parse().unwrap()` on a summary/bio value at any
      web/feed seam:
  - `rg -n 'summary(_label)?: *(Option<)?String' common/ storage/ server/ web/`
  - `rg -n 'bio: *(Option<)?String' storage/ web/`
  - `rg -n 'summary.*\.parse\(\)\.(expect|unwrap)|bio.*\.parse\(\)\.(expect|unwrap)' web/ server/ common/`
    Each must return only intentional/test hits (annotate any survivor).

- [ ] **Step 2: Full local gate** — `cargo xtask validate` (static + clippy +
      coverage + full `{sqlite,postgres}×{chromium,firefox}` e2e). Run
      **foreground** with `timeout: 600000` (per
      `project_gate_foreground_not_background`). Expected: green. Read
      `.xtask/last-result.json` `.steps` on any failure.

- [ ] **Step 3: Coverage burn-down check** — no new `cov:ignore` markers beyond
      host-uncoverable wasm-only `#[component]` render paths (ADR-0050
      exemption); the newtypes and `field_error` are host-covered.

- [ ] **Step 4** — hand off to **jaunder-ship** (final review, archive
      spec/plan, PR, merge).

---

## Self-Review

- **Spec coverage:** Newtypes + trailer + `truncated` (T1) ✓; Bio (T2) ✓;
  storage/feed/ render/web/form threading incl. every enumerated surface (T3–T4)
  ✓; ADR addendum (T1.S4) ✓; test_support helpers (T1/T2) ✓; fail-closed compat
  test (T3/T4) ✓; clear-path
  - disable-until-valid e2e (T5) ✓; no-parse-at-boundary + full-vertical grep
    (T6) ✓; `validate` green (T6) ✓. Spec criteria 1–9 all map.
- **Placeholder scan:** newtype bodies + tests written in full; threading tasks
  enumerate exact files:lines and provide the non-mechanical test code
  (fail-closed, field_error, e2e); no "TBD"/"handle edge cases".
- **Type consistency:** `PostSummary`/`Bio`/`InvalidPostSummary`/`InvalidBio`/
  `MAX_POST_SUMMARY_CHARS`/`MAX_BIO_CHARS`/`parse_post_summary`/`parse_bio`/
  `PostSummary::truncated`/`field.parsed()` used consistently across tasks.
