# Spec — issue #501: storage owned↔borrowed API shape fixes

**Issue:** jaunder-org/jaunder#501 — _storage: fix owned↔borrowed API shapes
forcing clones/re-parses_ **Milestone:** #13 Domain-value type safety (newtypes)

## Summary

An audit of `storage/`'s API surface found it already well-shaped (traits take
`&Newtype`, sqlx binds via `AsRef`/`Deref`) except a handful of localized
ownership defects. This cycle lands the three that are genuine, low-risk
improvements (#2–#4 below) and **descopes the original item #1** for the reason
recorded under "Descoped" — the design interview established it cannot be made
better without a worse trade.

## Scope

### AC-1 — `candidate_slug` returns `Slug`, not `String`

`storage/src/post_service.rs:403` `candidate_slug(&Slug, usize) -> String`
returns a known-valid string that the sole caller (`post_service.rs:492–495`)
immediately re-parses into `Slug`.

- Change the signature to
  `candidate_slug(slug_seed: &Slug, attempt: usize) -> Result<Slug, common::slug::InvalidSlug>`.
- attempt `0` returns `Ok(slug_seed.clone())` (already a valid `Slug`; no
  parse).
- The suffixed path builds the candidate string and parses **once** inside the
  function, returning that `Result`. The parse stays funnelled through
  `Slug::from_str` (the single validity chokepoint) — no bypass constructor.
- The caller at `:492` drops its own `.parse::<Slug>()` and instead propagates
  `candidate_slug(...)?` (mapping to `PerformCreationError::InvalidSlug` exactly
  as today).
- The unit tests at `post_service.rs:768–791` are updated to the new return
  type: `.unwrap()` the `Result` and assert on the `Slug` via `AsRef<str>`/
  equality. The residual `c.parse::<Slug>().is_ok()` re-validation asserts at
  `:777`/`:787` become redundant (the value is already a `Slug`) and are
  removed.
- Two duplicate tests in the `web` crate (`web/src/posts/mod.rs:842–852`, the
  only other `candidate_slug` consumer — test-only) are updated the same way;
  the signature change compels it.

**Observable:** the candidate-slug collision loop (`post_service.rs`
~`:491–495`) contains no `.parse::<Slug>()` — read the loop body to confirm (a
bare `rg 'parse::<Slug>' storage/src/post_service.rs` still matches unrelated
uses at `:326` and, pre-edit, the test asserts). The unit tests assert on a
returned `Slug`, not a bare `String`.

### AC-2 — `seed_post_input` takes `PostBody`, not `String`

`storage/src/post_service.rs:105` `seed_post_input(… body: String …)` converts
internally at `:115` (`body.into()`) where the `PostBody` newtype already
exists. This is `#[cfg(any(test, feature = "seed-posts"))]`-gated.

- Change the parameter to `body: PostBody` and pass it straight into
  `RenderedPostContent { body, … }` (drop the `.into()`).
- Update the seed/test callers to pass a `PostBody` (parsed via the project's
  newtype test-helper convention where the caller is `cfg(test)`; a plain
  `PostBody` value at seed sites).

**Observable:** `seed_post_input`'s signature names `body: PostBody`; no
`String`→`PostBody` conversion remains inside the function; the crate compiles
under both `--features seed-posts` and the default test build.

### AC-3 — bind post summary by borrow, not clone

`storage/src/posts.rs:1892` binds `input.summary.clone()` where
`input.summary: Option<String>`.

- Replace with `.bind(input.summary.as_deref())` (`Option<&str>`).

**Observable:** `rg 'summary.clone\(\)' storage/src/posts.rs` shows no
occurrence; the insert-path binding reads `input.summary.as_deref()`.

## Descoped

### Original item #1 — `hash_password`/`verify_password` taking `&Password`

**Not done, by design decision in the spec interview.** The issue proposed
having the hash helpers borrow `&Password` (and `hash: &str`) since "Argon2 only
reads." But both helpers offload the Argon2 work to
`tokio::task::spawn_blocking(move || …)` (`storage/src/helpers.rs:376,426`),
whose closure must be `'static` — so a borrow cannot be moved onto the blocking
pool. Every way to satisfy "no `Password` clone in storage" trades one downside
for another:

- **`block_in_place`** removes the clone but panics on a current-thread runtime,
  forcing every storage `#[tokio::test]` that transitively hashes to
  `flavor = "multi_thread"` — a runtime (not compile-time) failure mode, with no
  precedent in the repo.
- **Threading owned `Password` down** through `create_user`/`authenticate`/
  `set_password` removes the clone but breaks the "traits take `&Newtype`"
  convention the audit praised and ripples into server callers.
- **Copying the inner `String`** inside the helper is the same single allocation
  as today — a letter-of-the-law dodge with no real gain.

The `Password` is a short secret on cold auth paths, so the removed allocation
is negligible; none of the trades is worth it. Item #1 is left as-is and the
finding is reported back on the issue.

## Out of scope

- The other milestone-#13 conversion-audit issues (#498–#507); each is a
  separate cycle.
- Any change to `Password`, `Slug`, `PostBody`, or their newtype macros.
- Trait-signature changes (`create_user` etc. keep taking `&Password`).

## Acceptance (roll-up)

- No `Slug` re-parse in the candidate-slug path (AC-1).
- `seed_post_input` takes `PostBody` (AC-2).
- Post summary is bound by borrow, not clone (AC-3).
- `cargo xtask validate --no-e2e` is green (static + clippy + coverage).
- The original item #1 clone is intentionally retained, documented above and on
  the issue.
