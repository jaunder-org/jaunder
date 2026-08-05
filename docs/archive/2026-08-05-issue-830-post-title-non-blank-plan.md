# Blank `PostTitle` unrepresentable — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-08-05-issue-830-post-title-non-blank.md`](../specs/2026-08-05-issue-830-post-title-non-blank.md)
— the "what/why". This plan is the "how".

**Goal:** Move `PostTitle`'s non-blank invariant out of three convention-based
call sites and into the type, and replace `PostSummary::truncated`'s
`debug_assert` with a typed seed that proves non-blankness.

**Architecture:** Two compile-breaking changes, each landed whole because the
repo's gate requires every commit green. Task 2 flips `PostTitle` to the
validating kind and converts every construction site in the same commit (there
is no intermediate state — `From<String>` and `TryFrom<String>` collide via the
std blanket impl). Task 3 then introduces `SummarySeed` and retypes `truncated`,
which is independent of Task 2 except that `from_title`'s infallibility relies
on it.

**Tech Stack:** Rust, `macros::StrNewtype` derive, `cargo nextest`, `rstest`
(`#[apply(backends)]`), `cargo xtask check`.

## Review header

**Scope — in:** `common/src/post_title.rs`, `common/src/post_summary.rs`,
`common/src/test_support.rs`, `common/src/render.rs`,
`server/src/atompub/mapping.rs`, `server/tests/atompub/atompub_posts.rs`,
`storage/src/posts.rs`, `storage/src/test_support.rs`, fixture call sites, one
ADR draft.

**Scope — out:** `PostBody` (#811), a `PostTitle` length cap, `summary_label`
storage (#754), a general-purpose `NonBlankStr`, and what Atom emits for an
_untitled_ entry (Task 1 files it).

**Tasks:**

1. File the Atom untitled-`<title>` follow-up.
2. `PostTitle` becomes validating; every construction site converted.
3. `SummarySeed` + typed `PostSummary::truncated`; `fallback_summary_label`
   simplified.
4. ADR-0063 amendment draft.

**Key risks / decisions:**

- **Task 2 cannot be split.** Removing `#[str_newtype(infallible)]` deletes both
  `From<String>` and the `From<&str>` alias at once, so every construction site
  breaks in the same compile. Attempting a smaller first commit leaves the tree
  red and the gate will refuse it.
- **No production panic.** `render.rs:621` parses and _falls through to the
  untitled path_ on `Err` rather than `expect`ing — the invariant is provable
  but not to the compiler.
- **`from_title` is the only unbounded seed** (slug caps at 80, body line at
  100), so it is what keeps `MAX_POST_SUMMARY_CHARS` reachable. Re-seed the cap
  test with it; don't delete it.

## Global Constraints

- **Backend parity** — storage-touching tests use `#[apply(backends)]` +
  `#[case] backend: Backend`; a bare `#[tokio::test]` fails the
  `test-backend-pattern` guard.
- **Crate name is `jaunder`** — `cargo nextest run -p jaunder <filter>`. Bare
  nextest cannot run `case_2_postgres` locally (no PG daemon); filter to sqlite
  while iterating and let `cargo xtask check` cover both.
- **Per-commit gate** — run `cargo xtask check` before each commit
  (**`jaunder-commit`**). **No `Co-Authored-By` trailer.**
- **Suppressing a lint needs user approval** — fix the code instead.
- **ADR drafts are numberless** in `docs/adr/drafts/`; `cargo xtask adr promote`
  numbers them at ship.

---

### Task 1: File the Atom untitled-entry follow-up

`common/src/feed/atom.rs:32` renders `<title></title>` when a post has no title
(`unwrap_or_default()`). RFC 4287 requires `atom:entry/atom:title`, so omitting
it is not a free fix — the entry needs a substitute. Independent of this issue's
invariant.

**Files:** none (tracker-only).

- [x] **Step 1: File the issue** via **`jaunder-issues`** (its
      type/label/milestone conventions govern).

Title: `feed(atom): an untitled post renders <title></title>`

Body must state: `atom.rs:32` is
`Text::plain(i.title.clone().map(String::from).unwrap_or_default())`, so `None`
yields an empty `atom:title`. RSS is unaffected (`rss.rs:21` omits the element
on `None`). RFC 4287 makes `atom:title` mandatory, so the fix needs a decision —
permalink, summary label, or a literal like "(untitled)". Surfaced by #830's
spec review, which fixed only the `Some(PostTitle(""))` half.

- [x] **Step 2: Record the number.** Filed as
      [#832](https://github.com/jaunder-org/jaunder/issues/832) — type `Bug`,
      label `visibility`, milestone "Correctness & data integrity", P1 (protocol
      correctness: RFC 4287 §4.1.2 requires `atom:title`).

No commit — the tracker is the deliverable.

---

### Task 2: `PostTitle` becomes a validating newtype

**Files:**

- Modify: `common/src/post_title.rs` (whole file),
  `common/src/test_support.rs:241-246`, `common/src/render.rs:607-621`,
  `server/src/atompub/mapping.rs:86-90` and `:572`
- Fixtures: `storage/src/posts.rs:2954,2981,2996`, `storage/src/helpers.rs:552`,
  `server/tests/storage/mod.rs` (6 sites),
  `web/src/posts/{server,api,render}.rs` (9 sites),
  `server/src/atompub/posts.rs:564,628`
- Test: in-file `#[cfg(test)]` in `common/src/post_title.rs`; storage decode
  test in `storage/src/posts.rs`; `derive_post_title` unit test in
  `common/src/render.rs`; AtomPub blank-`<title>` test in
  `server/tests/atompub/atompub_posts.rs`. **`macros/` is not touched** — see
  AC7 below.

**Interfaces:**

- Produces: `PostTitle: FromStr<Err = InvalidPostTitle>` (+ derive-generated
  `TryFrom<String>`, validating serde/sqlx bridges). `From<String>` and
  `From<&str>` are **gone**.
  `common::test_support::parse_post_title(&str) -> PostTitle` remains, now
  validating.
- Consumes: nothing from Task 1.

- [x] **Step 1: Write the failing tests**

Replace `common/src/post_title.rs`'s test module with (keeping the existing
trimming and `Display` tests, adding rejection and wire tests):

```rust
    #[test]
    fn post_title_parses_and_trims_preserving_inner_and_case() {
        assert_eq!("  My Title  ".parse::<PostTitle>().unwrap(), "My Title");
        assert_eq!("a  b".parse::<PostTitle>().unwrap(), "a  b");
        assert_eq!("Москва".parse::<PostTitle>().unwrap(), "Москва");
    }

    #[test]
    fn post_title_rejects_empty_and_whitespace_only() {
        assert!("".parse::<PostTitle>().is_err());
        assert!("   ".parse::<PostTitle>().is_err());
        assert!("\t\n".parse::<PostTitle>().is_err());
    }

    #[test]
    fn post_title_serde_serializes_plain_string_and_validates_on_deserialize() {
        let t: PostTitle = "My Title".parse().unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"My Title\"");
        assert_eq!(
            serde_json::from_str::<PostTitle>("\"My Title\"").unwrap(),
            t
        );
        // AC2: blank is rejected on the wire, not coerced.
        assert!(serde_json::from_str::<PostTitle>("\"\"").is_err());
        assert!(serde_json::from_str::<PostTitle>("\"   \"").is_err());
    }
```

Add the decode test to `storage/src/posts.rs`, immediately after
`reading_post_with_overlong_summary_in_db_errors` (`:3437-3466`) and following
its idiom exactly — the seeding/accessor shapes below are the real ones,
verified against that test:

```rust
// AC3: a blank stored title fails the validating decode rather than being coerced.
// Forced in via raw SQL because the newtype can no longer construct one.
#[apply(backends)]
#[tokio::test]
async fn reading_post_with_blank_title_in_db_errors(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = SeedUser::new().seed(&env.state).await.user_id;
    let post_id = SeedRawPost::new(user_id).draft().seed(&env.state).await.post_id;

    env.base
        .pool()
        .execute(&*format!(
            "UPDATE posts SET title='' WHERE post_id={}",
            i64::from(post_id)
        ))
        .await
        .unwrap();

    let posts = &*env.state.posts;
    assert!(posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .is_err());
}
```

Four things the naive sketch gets wrong, called out so they aren't reintroduced:
`env.store` does **not** exist (it is `&*env.state.posts`); `get_post_by_id`
takes **two** args (the viewer identity); seeding returns a struct, so
`.user_id` / `.post_id` are needed; and `post_id` is an integer PK, so the
`WHERE` clause must not quote it.

Add the AC4 unit test to `common/src/render.rs`'s test module:

```rust
    #[test]
    fn derive_post_title_treats_blank_explicit_title_as_absent() {
        let (title, seed) =
            derive_post_title(Some("   "), "body line", &PostFormat::Markdown).unwrap();
        assert!(title.is_none());
        assert_eq!(seed, "body line");
    }
```

Add the AtomPub blank-`<title>` test to `server/tests/atompub/atompub_posts.rs`
— AC4's primary route, and the only one a user can actually submit a blank title
through. Follow that file's existing POST-an-entry idiom:

```rust
// AC4: a whitespace-only <title> means "no title", not a 400 — the presence policy
// now lives in PostTitle::from_str + ok(), not a hand-rolled guard (#830).
// Entry body supplies the slug seed.
```

Assert: the response is a success (201), and the created post has `title: None`
with a slug derived from the body — not a 400 and not a blank title.

**AC7 needs no macros-crate test.**
`validating_bridge_decodes_a_borrowed_str_without_allocating`
(`macros/src/str_newtype.rs:517-531`) already pins that the _validating kind_
decodes `&'r str` — it calls `sqlx_impls` with an arbitrary name, so it is
generic over the type. What it cannot prove is that `PostTitle` is **on** that
bridge, and the AC3 decode test above pins exactly that: a `title = ''` row can
only fail to decode if `Decode` routes through `FromStr`. Together they
discharge AC7, so `macros/` is **not** touched by this task.

**AC8 is discharged by AC1**, not by a step: once `Some(PostTitle(""))` is
unrepresentable, no titled entry can render an empty title.
`common/src/feed/atom.rs` and `rss.rs` are deliberately **untouched** — the
residual empty `<title>` for an _untitled_ entry is Task 1's follow-up.

- [x] **Step 2: Run the tests, verify they fail** — failed to compile at
      `render.rs:621` (`PostTitle::from` gone), the expected red.

Run: `devtool run --cwd <worktree> -- cargo nextest run -p common post_title`

Expected: FAIL — `PostTitle` has no `FromStr`, so the `.parse()` calls don't
compile. A compile failure is the expected red here, not an assertion failure.

- [x] **Step 3: Implement against the tests** — 21 fixture sites converted, all
      enumerated by `cargo check --workspace --all-targets`; matched the plan's
      inventory exactly, no site missed.

**3.1 — `common/src/post_title.rs`.** Replace the derive attribute and the door:

```rust
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A post's title: outer whitespace trimmed, non-empty. Casing and inner whitespace
/// are preserved (a title is human prose, not an identifier).
///
/// Constructed via [`FromStr`] — the single validating chokepoint, so a blank title
/// is unrepresentable rather than something call sites must remember to filter
/// (#830). An *absent* title is `None`: the field is `Option<PostTitle>` everywhere,
/// and blank input means absent (see `derive_post_title`).
///
/// **No length cap** — unlike `SessionLabel`, a title is unbounded prose; bounding it
/// is a separate decision. This makes `PostTitle` the only unbounded seed for
/// [`crate::post_summary::SummarySeed`].
///
/// `Hash` is retained from before this change; nothing currently hashes a
/// `PostTitle`, so ADR-0063 §2's "justify `Hash` per type" is unmet — but auditing it
/// is orthogonal to the invariant and is deliberately not bundled here. (Both peer
/// types, `SessionLabel` and `PostSummary`, carry an explicit "No `Hash`" note.)
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct PostTitle(String);

/// Error returned when a string cannot be parsed as a [`PostTitle`].
#[derive(Debug, Error)]
#[error("post title must be non-empty")]
pub struct InvalidPostTitle;

impl FromStr for PostTitle {
    type Err = InvalidPostTitle;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(InvalidPostTitle);
        }
        Ok(PostTitle(trimmed.to_owned()))
    }
}
```

Delete `#[str_newtype(infallible)]` and the `impl From<String> for PostTitle`.

**3.2 — `common/src/render.rs:620-622`.** Parse, falling through on `Err`:

```rust
    // `title` is non-blank by construction (the explicit branch is filtered above;
    // both extractors reject empty-after-trim), but the compiler can't see that —
    // so a failed parse falls through to the untitled path rather than panicking.
    if let Some(parsed) = title.as_deref().and_then(|t| t.parse::<PostTitle>().ok()) {
        return Some((Some(parsed), title.expect("checked above")));
    }
```

**Do not write this as a nested `if let`** —
`if let Some(t) = title { if let Ok(p) = … } }` is `clippy::collapsible_if`
shape, which is `-D warnings` here, and suppressing a lint needs user approval.
If the `expect` above reads badly, restructure with a `match` on the parse
result inside a single `if let Some(title) = title` — the constraint is one
nesting level, not a particular spelling. Whatever shape is chosen, **no
production panic on the `Err` path**: it falls through to the untitled branch.

Leave the `:607-609` filter in place and extend its comment to say it decides
_presence_ (whether the body is consulted at `:614`), not validity.

**3.3 — `server/src/atompub/mapping.rs:86-90`.** Let the type be the guard:

```rust
    // A blank <title> means the client supplied no title (#830): `FromStr` rejects
    // it, and `ok()` turns that into absence. No hand-rolled emptiness check — the
    // rule lives in `PostTitle`.
    let title = entry.title().as_str().parse::<PostTitle>().ok();
```

**3.4 — `common/src/test_support.rs:241-246`.** Mirror `parse_session_label`:

```rust
/// Build a [`PostTitle`] from `title` for tests — the single place a test title is
/// constructed. Panics on a blank title, which no test should be constructing.
#[must_use]
pub fn parse_post_title(title: &str) -> PostTitle {
    title.parse().expect("valid post title in test")
}
```

**3.5 — Fixtures.** Every `Some("…".into())` producing a `PostTitle`, and
`mapping.rs:572`'s `.map(PostTitle::from)`, become `parse_post_title("…")`.
AC9's grep-checkable form: no `PostTitle::from` and no `.into()` yielding a
`PostTitle` survives. Delete `storage/src/posts.rs:2979-2982` (case 2b) — the
state it builds is no longer representable.

- [x] **Step 4: Run the tests, verify they pass** — 14 targeted tests green,
      including both new blank-title tests.

Run: `devtool run --cwd <worktree> -- cargo nextest run -p common post_title`
then
`devtool run --cwd <worktree> -- cargo nextest run -p jaunder -E 'test(sqlite)'`

Expected: PASS, including
`fallback_summary_label_prefers_body_then_title_then_slug` (cases 1/2/3
unchanged) and `derive_post_title_allows_titleless_notes`.

- [x] **Step 5: Commit** — `086ed3e4`. The gate caught three lints on the first
      run: `clippy::doc_markdown` (`AtomPub` needing backticks),
      `missing_panics_doc` + `expect_used` on the `render.rs` parse — fixed by
      removing the `expect` entirely in favour of
      `if let Some((Ok(parsed), seed)) = title.map(…)`, which is what the plan
      wanted anyway — and `needless_borrow` on the mapping test helper. **The
      coverage risk did not materialise**: `coverage — clean`.

Run `cargo xtask check` first (**`jaunder-commit`**).

**Watch the coverage gate here.** This task deletes case 2b
(`storage/src/posts.rs:2979-2982`), which is the only test exercising the reject
branch of `fallback_summary_label`'s `.filter(|t| !t.trim().is_empty())` — and
that filter does not disappear until Task 3. Line coverage should survive (the
closure still runs on the accept path), but if the gate flags it, **fold Task
3's `fallback_summary_label` rewrite into this commit** rather than inventing a
synthetic test for a branch that is about to be deleted.

```bash
git add common/src/post_title.rs common/src/test_support.rs common/src/render.rs \
        server/src/atompub/mapping.rs server/src/atompub/posts.rs \
        server/tests/atompub/atompub_posts.rs \
        storage/src/posts.rs storage/src/helpers.rs server/tests/storage/mod.rs \
        web/src/posts
git commit -m "refactor(common): make a blank PostTitle unrepresentable (#830)"
```

---

### Task 3: `SummarySeed` and a typed `truncated`

**Files:**

- Modify: `common/src/post_summary.rs` (add `SummarySeed`, retype `truncated`,
  drop the `debug_assert`), `storage/src/posts.rs:100-122`,
  `storage/src/test_support.rs:1731`
- Test: in-file `#[cfg(test)]` in `common/src/post_summary.rs`

**Interfaces:**

- Consumes: `PostTitle: FromStr` from Task 2 (`from_title` is infallible only
  because a `PostTitle` is now non-blank).
- Produces: `SummarySeed` with `from_slug` / `from_title` / `first_body_line`;
  `PostSummary::truncated(&SummarySeed) -> PostSummary`.

- [x] **Step 1: Write the failing tests**

In `common/src/post_summary.rs`: **delete** `truncated_debug_asserts_non_empty`
(`:125-130`), and **replace** `truncated_trims_and_caps_at_char_boundary`
(`:116-123`) with the title-seeded cap test below.

To be precise about AC11's "kept and re-seeded": this is a rename **and a
narrowing**. The old test also asserted `truncated("  hi  ") == "hi"`; that trim
assertion is deliberately dropped, because trimming now happens in the seed
sources (`Slug`/`PostTitle` trim in their own `FromStr`; `first_body_line`
trims), so `truncated` no longer trims at all. The cap half — what AC11 exists
to preserve — is kept. Flagged so the checkbox isn't disputed at review.

Then add the seed tests:

```rust
    #[test]
    fn truncated_caps_at_char_boundary_from_a_title_seed() {
        // `from_title` is the only unbounded seed, so it is what keeps the cap
        // reachable (a slug caps at 80, a body line at 100).
        let long = parse_post_title(&"é".repeat(550));
        let summary = PostSummary::truncated(&SummarySeed::from_title(&long));
        assert_eq!(summary.chars().count(), MAX_POST_SUMMARY_CHARS);
    }

    #[test]
    fn first_body_line_finds_the_first_non_blank_line_and_caps_it() {
        let body = PostBody::from("\n\n   \n  hello  \nsecond\n".to_owned());
        let seed = SummarySeed::first_body_line(&body).unwrap();
        assert_eq!(PostSummary::truncated(&seed), "hello");

        let long = PostBody::from("x".repeat(500));
        let seed = SummarySeed::first_body_line(&long).unwrap();
        assert_eq!(PostSummary::truncated(&seed).chars().count(), 100);
    }

    #[test]
    fn first_body_line_is_none_for_a_blank_body() {
        assert!(SummarySeed::first_body_line(&PostBody::from("  \n\t\n".to_owned())).is_none());
    }
```

- [x] **Step 2: Run the tests, verify they fail** — failed to compile,
      `SummarySeed` undeclared, as expected.

Run: `devtool run --cwd <worktree> -- cargo nextest run -p common post_summary`

Expected: FAIL to compile — `SummarySeed` does not exist.

- [x] **Step 3: Implement against the tests**

In `common/src/post_summary.rs` — note the file currently imports **none** of
these three types, so add them (same crate, no cycle):

```rust
use crate::post_body::PostBody;
use crate::post_title::PostTitle;
use crate::slug::Slug;
```

(Match the crate's actual module paths/`use` style as written in
`common/src/lib.rs`.) Then:

```rust
/// A summary label that is already known non-blank.
///
/// Its constructors are infallible because each *source* proves the invariant: a
/// [`Slug`] and a [`PostTitle`] are non-blank by construction, and a body line is
/// selected only when non-blank. This replaces the `debug_assert` that
/// [`PostSummary::truncated`] used to carry — the precondition is now a type, so a
/// caller cannot forget it (#830).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummarySeed(String);

/// How much of a body line may seed a summary, in scalars.
const MAX_BODY_LINE_SEED_CHARS: usize = 100;

impl SummarySeed {
    /// A slug is non-blank by construction (`Slug::from_str` rejects empty).
    #[must_use]
    pub fn from_slug(slug: &Slug) -> Self {
        Self(slug.to_string())
    }

    /// A [`PostTitle`] is non-blank by construction (#830).
    #[must_use]
    pub fn from_title(title: &PostTitle) -> Self {
        Self(title.to_string())
    }

    /// The first non-blank line of `body`, capped at [`MAX_BODY_LINE_SEED_CHARS`].
    /// `None` when the body has no such line — a real domain answer (an empty post),
    /// not a validation failure.
    #[must_use]
    pub fn first_body_line(body: &PostBody) -> Option<Self> {
        body.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| Self(line.chars().take(MAX_BODY_LINE_SEED_CHARS).collect()))
    }
}
```

Retype `truncated` and delete its `debug_assert` (the seed is already trimmed
and non-blank, so the trim goes too — state that in the doc comment):

```rust
    /// Length-caps an already-non-blank [`SummarySeed`] into a `PostSummary`.
    ///
    /// Infallible: the seed proves non-blankness by construction, and over-length is
    /// the one thing this door coerces rather than rejects. See #564 for word-aware
    /// cutting.
    #[must_use]
    pub fn truncated(seed: &SummarySeed) -> Self {
        Self(seed.0.chars().take(MAX_POST_SUMMARY_CHARS).collect())
    }
```

`storage/src/posts.rs:100-122` becomes a seed selection — the emptiness filter,
the inlined line search, and the "invariant gap" comment all go:

```rust
    /// Generates a fallback summary from the post's body, title, or slug. Every
    /// candidate is non-blank by construction, so the chain cannot yield an empty
    /// label and `truncated` needs no emptiness check.
    pub fn fallback_summary_label(&self) -> PostSummary {
        let seed = SummarySeed::first_body_line(&self.body)
            .or_else(|| self.title.as_ref().map(SummarySeed::from_title))
            .unwrap_or_else(|| SummarySeed::from_slug(&self.slug));
        PostSummary::truncated(&seed)
    }
```

`storage/src/test_support.rs:1731` becomes
`.summary(PostSummary::truncated(&SummarySeed::from_title(&parse_post_title("excerpt"))))`.

- [x] **Step 4: Run the tests, verify they pass** — 41 summary/seed tests green.

Run: `devtool run --cwd <worktree> -- cargo nextest run -p common post_summary`
then
`devtool run --cwd <worktree> -- cargo nextest run -p jaunder -E 'test(sqlite)'`

Expected: PASS — including
`fallback_summary_label_prefers_body_then_title_then_slug` (body → title → slug
precedence unchanged) and `draft_row_falls_back_to_summary_label_when_untitled`.

- [x] **Step 5: Commit** — `977f345f`. Gate caught two lints first: an unused
      top-level `SummarySeed` import (its only use is inside `#[cfg(test)]`, so the
      import moved into that module) and a missing `#[must_use]` on
      `fallback_summary_label`.

```bash
git add common/src/post_summary.rs storage/src/posts.rs storage/src/test_support.rs
git commit -m "refactor(common): type PostSummary::truncated's non-blank precondition (#830)"
```

---

### Task 4: ADR-0063 amendment draft

**Files:** Create `docs/adr/drafts/<slug>.md` per **`jaunder-adr`**
(numberless).

- [x] **Step 1: Write the draft**

It must make two corrections:

1. **§3's infallible-kind definition** — currently "a value whose invariant
   never rejects (only normalizes, or wraps verbatim)", a test on the
   _constructor's signature_, which contradicts §2's invariant-first framing
   ("fallible **when the value has an invariant**"). That mismatch is what let
   `PostTitle` be mislabelled. Restate it invariant-first, and drop **both**
   `PostTitle` and `PostBody` from the "first users" list — `PostTitle` by this
   issue, `PostBody` by #811. State plainly that `PostBody` is still infallible
   in code until #811 lands, and that this amendment discharges #811's ADR
   obligation.
2. **§2's truncating-door paragraph (`:281-294`)** — it describes `truncated` as
   a trust door guaranteeing "only the cap, not [non-emptiness]", cites the
   `debug_assert!` as the mechanism, and names `PostSummary` (#545) as first
   user. Replace with the typed-seed pattern: where the caller can supply a
   value whose _source_ proves the non-length half of the invariant, a seed type
   carries that proof and the trust door becomes a plain length-capping door.

- [x] **Step 2: Verify format and links**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS —
`adr-format`, `adr-readme-parity`, and `doc-links` all green. Note these gates are
**blind to drafts by construction** (`docs/adr/drafts/README.md` §"Gate
invisibility"): the first three enumerate `docs/adr/` non-recursively and require a
leading number, and `doc-links` walks tracked files only. So a green here does not
validate the draft; the real check is `promote`'s output at ship.

- [x] **Step 3: No commit — the draft is gitignored.**

**Correction to this plan as written.** Task 4 originally said to
`git add docs/adr/drafts`, which cannot work: everything in that directory except its
`README.md` is gitignored, deliberately, so a draft cannot be committed with a
premature number (#219's draft-out-of-git flow). The draft stays uncommitted through
this cycle. At ship, **after** the final rebase onto `main`,
`cargo xtask adr promote` assigns the next free number, moves the file to
`docs/adr/NNNN-<slug>.md`, rewrites its path-form references, flips `proposed` →
`accepted`, syncs the README table, and stages the result — and *that* is what gets
committed. The ADR's first appearance in git history is already correctly numbered.

Draft written: `docs/adr/0101-infallible-kind-is-invariant-first.md`.

---

## Verification

- [ ] `devtool run --cwd <worktree> -- cargo xtask validate` green (static +
      clippy + coverage + all four `{sqlite,postgres}×{chromium,firefox}` e2e
      combos).
- [x] `rg 'PostTitle::from' -g '*.rs'` returns nothing (AC9). Scope to `*.rs` —
      an unscoped search matches this plan, the spec, and ~15 files under
      `docs/archive/`. This covers only half of AC9; the "no `.into()` yielding
      a `PostTitle`" half is enforced by the compiler, since `From<String>` no
      longer exists.
- [x] `rg 'debug_assert' common/src/post_summary.rs` returns no **assertion**
      (AC5). One prose match remains, in the `SummarySeed` doc comment explaining
      what the seed type replaced — documentation of the removal, not the thing.
- [x] `rg 'invariant gap' storage/src/posts.rs` returns nothing (AC6).
