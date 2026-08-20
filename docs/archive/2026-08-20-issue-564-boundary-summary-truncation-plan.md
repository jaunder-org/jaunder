# Boundary-aware derived Post summaries Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make derived Post summary labels and untitled slug seeds cut at
sentence/word boundaries instead of raw scalar counts.

**Architecture:** Put the boundary-cut algorithm in one `common` helper and
route every derived-text cap through it. Remove `SummarySeed` unless
implementation proves it still clarifies the non-blank invariant; the default
plan removes it and replaces the two-stage body pipeline with explicit
derived-summary constructors on `PostSummary`.

**Tech Stack:** Rust `common` crate unit tests; `storage` PostRecord tests;
`cargo nextest`; repository gate via `devtool run -- cargo xtask check`.

## Global Constraints

- Approved spec:
  `docs/superpowers/specs/2026-08-20-issue-564-boundary-summary-truncation-spec.md`.
- Do not change submitted `PostSummary::from_str` validation: over-cap submitted
  summaries still fail.
- Do not change `MAX_POST_SUMMARY_CHARS` or `MAX_BODY_LINE_SEED_CHARS`.
- No HTML/Markdown/Org sentence parsing; this is a plain-text heuristic over the
  already-selected seed string.
- No synthetic ellipsis.
- Every derived-text cap in scope must use one shared boundary-truncation
  helper.
- `SummarySeed` must not remain as a confusing two-stage cap pipeline; either
  document a clear proof role or remove it.
- No `Co-Authored-By` trailer.

---

## Review header

**Scope in**

- `common/src/post_summary.rs`: shared boundary helper, derived-summary
  construction, tests.
- `common/src/render.rs`: untitled body-line slug seed uses the shared helper,
  tests.
- `storage/src/posts.rs`: fallback summary call site and test naming/comments.
- `storage/src/test_support.rs` only if the `SummarySeed` removal touches its
  seed fixture.
- `common/src/post_title.rs` docs only if `SummarySeed` is removed and its
  reference must be rewritten.

**Scope out**

- User-submitted summary behavior.
- New fields, storage migrations, background jobs, UI changes, or e2e tests.
- Renderer-aware excerpt parsing.

1. Replace `SummarySeed` pipeline with explicit derived-summary constructors,
   one boundary helper, and updated fallback-summary call sites.
2. Route `derive_post_naming`'s untitled body-line seed through the shared
   helper.
3. Verify no duplicate truncation path remains and run the full per-commit gate.

**Key risks / decisions**

- `PostTitle` is unbounded, so title-derived summaries still need the 500-scalar
  cap.
- Body-line summaries cap at 100 before becoming a `PostSummary`; do not leave a
  second 500-cap no-op on that route.
- Keep helper `pub(crate)` in `common::post_summary` so `common::render` can use
  it without making a public API promise.
- Prefer removing `SummarySeed`: current production use is only
  `storage::PostRecord::fallback_summary_label`; `from_slug` is test-only;
  `storage/src/test_support.rs` can use an explicit title-derived constructor or
  a parsed short summary.

## File structure

- `common/src/post_summary.rs`
  - Owns constants, submitted `PostSummary::FromStr`, and derived `PostSummary`
    constructors.
  - Add
    `pub(crate) fn truncate_at_text_boundary(input: &str, max_scalars: usize) -> String`.
  - Remove `SummarySeed` unless the implementation finds a stronger invariant
    role than the current two-stage cap pipeline.
- `common/src/render.rs`
  - Imports/uses `truncate_at_text_boundary` for `first_meaningful_line`'s
    100-scalar cap.
- `storage/src/posts.rs`
  - Uses `PostSummary::from_body_line(&self.body)` (exact name below) for
    fallback labels.
- `storage/src/test_support.rs`
  - Replace `PostSummary::truncated(&SummarySeed::from_title(...))` with
    `PostSummary::from_title(...)` or a direct parsed `PostSummary` if the
    fixture does not need truncation semantics.
- `common/src/post_title.rs`
  - Remove/replace the rustdoc reference to `SummarySeed` if the type is
    removed.

---

### Task 1: Derived-summary construction and shared boundary helper

**Files:**

- Modify: `common/src/post_summary.rs:1-227`
- Modify: `common/src/post_title.rs:19-22`
- Modify: `storage/src/test_support.rs:1549-1795`
- Modify: `storage/src/posts.rs:100-109,3292-3323`
- Test: `common/src/post_summary.rs`
- Test: `storage/src/posts.rs`

**Interfaces:**

- Produces:
  `pub(crate) fn truncate_at_text_boundary(input: &str, max_scalars: usize) -> String`
  - `input`: already selected plain-text seed.
  - `max_scalars`: cap in Unicode scalar values.
  - Returns unchanged input when `input.chars().count() <= max_scalars`.
  - Otherwise returns the non-empty prefix chosen by: last `.`, `!`, or `?`
    within cap; else last Unicode whitespace boundary within cap; else hard
    scalar cap. Trim trailing whitespace after the chosen cut. Never byte-slice
    through UTF-8.
- Produces: `impl PostSummary { pub fn from_title(title: &PostTitle) -> Self }`
  - Uses `truncate_at_text_boundary(title.as_ref(), MAX_POST_SUMMARY_CHARS)`.
- Produces:
  `impl PostSummary { pub fn from_body_line(body: &PostBody) -> Self }`
  - Selects the first non-blank body line, trims trailing line whitespace, caps
    with `truncate_at_text_boundary(..., MAX_BODY_LINE_SEED_CHARS)`, constructs
    `PostSummary` directly.
  - It is infallible because `PostBody` guarantees at least one non-blank line.
- Removes: `SummarySeed`, `SummarySeed::from_slug`, `SummarySeed::from_title`,
  `SummarySeed::first_body_line`, and `PostSummary::truncated` unless
  implementation finds a documented proof role that avoids the two-stage no-op
  body cap. The default is removal.
- Consumed by later tasks: `truncate_at_text_boundary` in `common::render`;
  `PostSummary::from_body_line` in
  `storage::PostRecord::fallback_summary_label`.

- [x] **Step 1: Write failing `PostSummary` tests**

Replace the existing `truncated_caps_at_char_boundary_from_a_title_seed` and
`first_body_line_finds_the_first_non_blank_line_and_caps_it` tests with these
contracts:

```rust
#[test]
fn derived_title_summary_prefers_sentence_boundary() {
    let prefix = "A complete sentence.";
    let filler = format!(" {}", "word".repeat(MAX_POST_SUMMARY_CHARS));
    let title: PostTitle = format!("{prefix}{filler}").parse().unwrap();

    assert_eq!(PostSummary::from_title(&title), prefix);
}

#[test]
fn derived_title_summary_falls_back_to_word_boundary() {
    let title: PostTitle = format!("{} finalword", "word ".repeat(MAX_POST_SUMMARY_CHARS / 5))
        .parse()
        .unwrap();
    let summary = PostSummary::from_title(&title);

    assert!(summary.chars().count() <= MAX_POST_SUMMARY_CHARS);
    assert!(!summary.ends_with("finalword"));
    assert!(!summary.ends_with(' '));
}

#[test]
fn derived_title_summary_hard_caps_one_long_token_without_splitting_utf8() {
    let title: PostTitle = "é".repeat(MAX_POST_SUMMARY_CHARS + 50).parse().unwrap();
    let summary = PostSummary::from_title(&title);

    assert_eq!(summary.chars().count(), MAX_POST_SUMMARY_CHARS);
    assert!(summary.chars().all(|c| c == 'é'));
}

#[test]
fn derived_body_summary_prefers_boundary_within_body_line_cap() {
    let body = crate::test_support::parse_post_body(&format!(
        "{} trailingword\nsecond line",
        "body word ".repeat(MAX_BODY_LINE_SEED_CHARS / 10)
    ));
    let summary = PostSummary::from_body_line(&body);

    assert!(summary.chars().count() <= MAX_BODY_LINE_SEED_CHARS);
    assert!(!summary.ends_with("trailingword"));
    assert!(!summary.ends_with(' '));
}

#[test]
fn submitted_over_cap_summary_still_rejects() {
    let over = "a".repeat(MAX_POST_SUMMARY_CHARS + 1);

    assert!(over.parse::<PostSummary>().is_err());
}
```

Keep the existing tests for no trailing newline and CRLF behavior, but route
them through `PostSummary::from_body_line(&body)` instead of `SummarySeed` +
`PostSummary::truncated`.

- [x] **Step 2: Run the new common tests and verify failure**

Run:

```bash
devtool run -- cargo nextest run -p common post_summary
```

Expected: FAIL because `PostSummary::from_title`, `PostSummary::from_body_line`,
and boundary-aware behavior do not exist yet.

- [x] **Step 3: Implement the shared helper and constructors**

Implement in `common/src/post_summary.rs`:

```rust
pub(crate) fn truncate_at_text_boundary(input: &str, max_scalars: usize) -> String {
    if input.chars().count() <= max_scalars {
        return input.to_owned();
    }

    let hard_cap: String = input.chars().take(max_scalars).collect();
    let sentence = hard_cap
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| matches!(ch, '.' | '!' | '?').then_some(idx + ch.len_utf8()))
        .and_then(|end| non_empty_trimmed_prefix(&hard_cap[..end]));
    if let Some(prefix) = sentence {
        return prefix;
    }

    let word = hard_cap
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| ch.is_whitespace().then_some(idx))
        .and_then(|end| non_empty_trimmed_prefix(&hard_cap[..end]));
    if let Some(prefix) = word {
        return prefix;
    }

    hard_cap
}

fn non_empty_trimmed_prefix(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
```

Then implement the two constructors exactly as listed in **Interfaces**. Remove
`SummarySeed` and `PostSummary::truncated`; update module rustdoc to say the
internal derived doors are `from_title` and `from_body_line`. Update
`common/src/post_title.rs` lines 19-22 to refer to `PostSummary::from_title`
rather than `SummarySeed`.

Update `storage/src/test_support.rs` to remove
`use common::post_summary::SummarySeed;` and replace the fixture summary
construction with either:

```rust
.summary(PostSummary::from_title(&parse_post_title("excerpt")))
```

or, if the fixture does not need derived-title semantics:

```rust
.summary(parse_post_summary("excerpt"))
```

Prefer `PostSummary::from_title` here because it keeps one non-test caller
exercising the title-derived constructor.

- [x] **Step 4: Run the targeted common/storage tests and verify pass**

Run:

```bash
devtool run -- cargo nextest run -p common post_summary
```

Expected: PASS.

Run:

```bash
devtool run -- cargo nextest run -p storage fallback_summary_label_uses_the_first_non_blank_body_line
```

Expected: PASS.

- [x] **Step 5: Run a compile check for downstream callers**

Run:

```bash
devtool run -- cargo check -p jaunder -p storage
```

Expected: PASS. Any `SummarySeed`/`PostSummary::truncated` compile failure means
a stale caller remains; fix it before continuing.

- [x] **Step 6: Stage, gate, and commit Task 1**

Tick this task's plan checkboxes, then stage exactly what will land before
running the gate:

```bash
git add common/src/post_summary.rs common/src/post_title.rs storage/src/test_support.rs storage/src/posts.rs docs/superpowers/plans/2026-08-20-issue-564-boundary-summary-truncation-plan.md
```

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. If the gate applies mechanical fixes, inspect them, stage the
intended fixes, and rerun `devtool run -- cargo xtask check` until it passes on
the staged tree. Then commit the already-checked staged tree:

```bash
git commit -m "refactor(common): simplify derived post summary construction (#564)"
```

No `Co-Authored-By` trailer.

---

### Task 2: Render slug seed uses the shared boundary helper

**Files:**

- Modify: `common/src/render.rs:574-637,960-1128`
- Test: `common/src/render.rs`

**Interfaces:**

- Consumes:
  `common::post_summary::truncate_at_text_boundary(input: &str, max_scalars: usize) -> String`
  from Task 1.
- Produces: `first_meaningful_line(body: &PostBody) -> String` still returns the
  trimmed first non-blank body line capped at 100 Unicode scalar values, but now
  uses the shared boundary helper.
- Preserves: `derive_post_naming(None, body, format)` remains total and still
  falls back to slug `post` when the selected seed has no slug characters.

- [x] **Step 1: Write failing render tests**

Add tests near existing `derive_post_naming_*` tests:

```rust
#[test]
fn derive_post_naming_untitled_slug_seed_prefers_word_boundary() {
    let body = format!("{}trailingword\nsecond line", "slug     ".repeat(10));
    let (title, slug) = naming(None, &body, PostFormat::Html);

    assert_eq!(title, None);
    assert_eq!(
        slug.as_ref(),
        [
            "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug",
        ]
        .join("-")
    );
}

#[test]
fn derive_post_naming_untitled_slug_seed_hard_caps_long_token() {
    let body = format!("{}\nsecond line", "é".repeat(150));
    let (title, slug) = naming(None, &body, PostFormat::Html);

    assert_eq!(title, None);
    assert_eq!(slug.as_ref(), "é".repeat(crate::slug::MAX_SLUG_CHARS));
}
```

The exact expected slug is part of the contract: the old raw 100-scalar cut
includes part of `trailingword`, while the shared helper backs up to the
previous word boundary.

- [x] **Step 2: Run the render tests and verify failure**

Run:

```bash
devtool run -- cargo nextest run -p common derive_post_naming_untitled_slug_seed
```

Expected: FAIL under the current raw `line.chars().take(100)` behavior for the
word-boundary test.

- [x] **Step 3: Implement render call-through**

Modify `common/src/render.rs`:

```rust
use crate::post_summary::truncate_at_text_boundary;
```

Change `first_meaningful_line` to:

```rust
fn first_meaningful_line(body: &PostBody) -> String {
    let Some(line) = body.lines().map(str::trim).find(|line| !line.is_empty()) else {
        unreachable!("a PostBody always has a non-blank line")
    };
    truncate_at_text_boundary(line, 100)
}
```

Do not create a second render-specific truncation helper.

- [x] **Step 4: Run the targeted render tests and verify pass**

Run:

```bash
devtool run -- cargo nextest run -p common derive_post_naming
```

Expected: PASS.

- [x] **Step 5: Stage, gate, and commit Task 2**

Tick this task's plan checkboxes, then stage exactly what will land before
running the gate:

```bash
git add common/src/render.rs docs/superpowers/plans/2026-08-20-issue-564-boundary-summary-truncation-plan.md
```

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. If the gate applies mechanical fixes, inspect them, stage the
intended fixes, and rerun `devtool run -- cargo xtask check` until it passes on
the staged tree. Then commit the already-checked staged tree:

```bash
git commit -m "fix(common): boundary-truncate untitled slug seeds (#564)"
```

No `Co-Authored-By` trailer.

---

### Task 3: Duplicate-path audit and final verification

**Files:**

- Inspect: `common/src/post_summary.rs`, `common/src/render.rs`,
  `common/src/post_title.rs`, `storage/src/test_support.rs`,
  `storage/src/posts.rs`
- Modify: only the approved spec/plan checkboxes if executing under
  `jaunder-iterate`

**Interfaces:**

- Consumes: `PostSummary::from_body_line(&PostBody) -> PostSummary` from Task 1.
- Consumes: `common::post_summary::truncate_at_text_boundary` from Tasks 1-2.
- Verifies: no production or test caller still imports or references
  `SummarySeed` or `PostSummary::truncated` after the chosen simplification.

- [x] **Step 1: Verify storage fallback behavior**

Run:

```bash
devtool run -- cargo nextest run -p storage fallback_summary_label_uses_the_first_non_blank_body_line
```

Expected: PASS. This proves `PostRecord::fallback_summary_label` remains total
on `PostBody`, uses no title/slug fallback, and observes the boundary-aware body
line cap through the public storage method.

- [x] **Step 2: Verify no duplicate old API remains**

Use repo search, not shell grep:

- Search pattern:
  `SummarySeed|PostSummary::truncated|chars\(\)\.take\(MAX_BODY_LINE_SEED_CHARS\)|chars\(\)\.take\(100\)`
- Expected remaining matches:
  - constants/rustdoc that are still true;
  - no `SummarySeed` type/import/caller;
  - no raw `chars().take(100)` or `chars().take(MAX_BODY_LINE_SEED_CHARS)` in
    the derived summary/render paths.

If `SummarySeed` was deliberately kept instead of removed, the remaining matches
must be limited to its documented proof role and must not include a two-stage
cap where body-line seeds are capped at 100 then capped again at 500.

- [x] **Step 3: Run targeted crate tests**

Run:

```bash
devtool run -- cargo nextest run -p common post_summary
```

Expected: PASS.

Run:

```bash
devtool run -- cargo nextest run -p common derive_post_naming
```

Expected: PASS.

Run:

```bash
devtool run -- cargo nextest run -p storage fallback_summary_label_uses_the_first_non_blank_body_line
```

Expected: PASS.

- [x] **Step 4: Stage, gate, and commit Task 3**

Tick this task's plan checkboxes, then stage all remaining issue files,
including the approved spec and this plan if not already staged:

```bash
git add docs/superpowers/specs/2026-08-20-issue-564-boundary-summary-truncation-spec.md docs/superpowers/plans/2026-08-20-issue-564-boundary-summary-truncation-plan.md storage/src/posts.rs common/src/post_summary.rs common/src/post_title.rs common/src/render.rs storage/src/test_support.rs
```

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. If the gate applies mechanical fixes, inspect them, stage the
intended fixes, and rerun `devtool run -- cargo xtask check` until it passes on
the staged tree. Then commit the already-checked staged tree:

```bash
git commit -m "docs(superpowers): plan boundary summary truncation (#564)"
```

If all code changes were already committed in Tasks 1-2 and only docs remain,
use the docs commit message above. If Task 3 unexpectedly contains code fixes,
use:

```bash
git commit -m "fix(common): finish boundary truncation cleanup (#564)"
```

No `Co-Authored-By` trailer.

## Self-review

- Spec coverage: every acceptance criterion maps to Task 1 (`PostSummary`,
  helper, submitted rejection), Task 2 (`derive_post_naming` slug seed), or Task
  3 (`storage fallback totality and no old duplicate API`).
- Placeholder scan: no TBD/TODO/fill-in placeholders; test contracts and
  signatures are explicit.
- Type consistency: `truncate_at_text_boundary`, `PostSummary::from_title`, and
  `PostSummary::from_body_line` are introduced before later tasks consume them.
- Scope: no UI, storage schema, e2e, renderer-aware parsing, or
  submitted-summary behavior changes.

Plan complete and saved to
`/home/mdorman/src/jaunder/agent-5/docs/superpowers/plans/2026-08-20-issue-564-boundary-summary-truncation-plan.md`.
