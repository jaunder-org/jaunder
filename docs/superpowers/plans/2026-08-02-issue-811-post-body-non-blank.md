# Implementation plan — #811: make a blank `PostBody` unrepresentable

**Issue:** [#811](https://github.com/jaunder-org/jaunder/issues/811) — the issue
carries the what/why and the five decisions; this plan is the how. Don't
re-derive the analysis from it.

**Branch:** `worktree-issue-811-post-body-non-blank`. It was stacked on
`worktree-issue-569-post-dto-names` (PR #810); **#810 merged 2026-08-03** and
this branch has been rebased onto `main`, so the stack is gone. Line numbers
below were re-verified against the rebased tree on 2026-08-05.

**For agentic workers:** drive with `jaunder-iterate`; delegate individual tasks
via `jaunder-dispatch`. Tick checkboxes in real time.

---

## Review header

**Goal.** A post body with no non-blank line becomes unrepresentable rather than
rejected downstream. Everything else in this plan follows from that: the
emptiness gate moves into the type, so `derive_post_title` loses its `Option`,
canonicalization becomes a typed fallible seam for both formats, and a test-only
duplicate of the update write path is deleted.

**Scope — in.** `PostBody`'s construction doors; `canonicalize_org_body` and a
matching Markdown seam; `derive_post_title`'s signature; `EmptyPost`'s home;
deleting `update_rendered_post`; the ADR amendment; the web/AtomPub/seed call
sites and tests that construct bodies.

**Scope — out.** The `#810` DTO renaming (already landed). Slug generation
itself (ADR-0025 unchanged). Any change to `RenderedHtml`. Media extraction.
#797's `parse_post_cursor` removal — separate issue, not blocked by this.
**#830's `PostTitle` invariant** — the sibling issue; this branch touches
ADR-0063 §3 in a way that must serve both (task 1), but does not change
`PostTitle`.

**Tasks.**

1. ADR amendment: `PostBody` carries a non-blank invariant (supersedes the #402
   note).
2. `PostBody` gains a validating `FromStr`; `infallible` comes off. One door.
3. Fix the fallout at every `PostBody` construction site so the tree compiles.
4. `canonicalize_org_body`: `PostBody -> Result<PostBody, _>`.
5. One `canonicalize_body` seam over all formats, carrying the agreed whitespace
   normalization; `post_service` stops special-casing Org.
6. Title-only Org post is rejected (behaviour change), pinned by a test.
7. `derive_post_title` goes total: `(Option<PostTitle>, Slug)`; `EmptyPost`
   retires from the service layer.
8. Delete `update_rendered_post`; delete or retarget its three tests.
9. Confirm-or-remove `InvalidSlug` on the derived path.
10. Close #785; sweep for stragglers; full gate.

**Key risks and decisions.**

- **Behaviour change (task 6).** A title-only Org post (`* My Title`, no
  content) is accepted today and becomes a 400. Decided deliberately — issue
  #811 decision 2. This is the one change a user could notice; it needs its own
  test and a line in the PR body.
- **One door, including sqlx (task 2).** Decided — no `from_trusted` bypass, no
  legacy data to accommodate. Consequence: a blank body row in a database would
  fail to _decode_. Accepted because no such data exists.
- **Whitespace: never blanket-`trim()`, but leading blank lines and trailing
  whitespace _are_ normalizable.** All figures measured 2026-08-05 against the
  real renderers; the scratch probe is deleted, so the numbers below are the
  record.

  **What a blanket trim breaks.** `"    fn main() {}\n"` as `Markdown`:

  ```
  untrimmed => <pre><code>fn main() {}\n</code></pre>
  trimmed   => <p>fn main() {}</p>
  ```

  Four leading spaces is a CommonMark indented code block; trimming turns a code
  block into prose. It reaches users unfiltered — `RenderOutput::render`
  (`render.rs:325`) hands the body straight to `render_markdown`, and today only
  Org is canonicalized in storage (`post_service.rs:317,484`).

  **What is safe** (decided with the maintainer, 2026-08-05): strip **leading
  all-whitespace lines**, `trim_end()`, then **re-append one `\n`**. Never strip
  leading _horizontal_ whitespace on a line that has content; never touch
  interior blank lines. Byte-identical rendering on every Markdown and Org case
  measured except the one below.

  **The accepted divergence.** `trim_end` is lossy when the body ends _inside an
  unclosed code region_, where trailing blank lines are content:

  ````
  body = "```\ncode\n\n"      (unclosed fence)
    raw        => <pre><code>code\n\n</code></pre>
    trim_end'd => <pre><code>code\n</code></pre>
  ````

  Detecting this needs a format parser, not a whitespace rule. Accepted: the
  input is malformed, and the loss is trailing blank lines inside it.

  **Why the `\n` restore is not optional.** Bare `trim_end()` also eats the
  body's _terminating_ newline, which is significant inside `<pre><code>` and
  inside Org paragraphs — that alone caused 6 of the 8 failures measured. Note
  today's `canonicalize_org_body` does _not_ restore it, so this fixes a latent
  Org bug while generalizing the rule.

  **Interior blank lines are load-bearing — do not "tidy" them.** They control
  CommonMark loose-vs-tight lists:

  ```
  "- a\n\n- b\n" => <ul><li><p>a</p></li><li><p>b</p></li></ul>
  "- a\n- b\n"   => <ul><li>a</li><li>b</li></ul>
  ```

  **HTML is exempt.** `PostFormat::Html` is verbatim passthrough, so _any_
  whitespace edit is a byte change, and the unclosed-`<pre>` case fails exactly
  like the fence. Leave it alone.

  Round-trip is _not_ part of this justification — the Emacs design doc's
  "verbatim" commitments concern unrecognized header lines, and the content ETag
  hashes the body as stored, so a consistently applied normalization breaks
  neither.

  Note `post_body_wraps_verbatim_without_trimming` is **not** adequate evidence
  for any of this — it arrived in `8d339c70` ("types(common): add PostBody and
  PostTitle newtypes (#402)") with no rationale beyond its own comment. Keep it;
  the load-bearing guards are the render-level tests in tasks 2 and 5.

- **Layering: `PostBody` does not know its format, so normalization cannot live
  in `from_str`.** Format lives beside the body as `PostFormat`. This forces —
  and the forcing is good — a two-layer split:
  - `PostBody::from_str` (format-agnostic): reject a body with no non-blank
    line; store **verbatim**. That is the invariant, and it is what makes the
    sqlx/serde decode door safe and idempotent.
  - `canonicalize_body(&PostBody, &PostFormat)` (task 5, format-aware): the
    whitespace normalization above, plus Org's title-source stripping, plus
    HTML's exemption.

    Consequence: the stored body is the canonicalized one, and re-decoding it
    only re-checks non-blankness — so the two layers compose without a second
    normalization pass. Verified: `canonicalize_org_body` is idempotent, and
    `canonicalize_org_body(normalize(x)) == canonicalize_org_body(x)` on every
    input tested.

- **Markdown bodies become normalized on write — a new, if minor, behaviour
  change.** Markdown is stored verbatim today, so this is the first time stored
  bytes differ from submitted bytes on that path: a body with leading blank
  lines or trailing whitespace gets a different content ETag, and the AtomPub
  round-trip stops being byte-identical for it. Org bodies also shift slightly
  (they regain a terminating `\n`). Both belong in the PR body beside the
  title-only-Org change, not landing silently.

- **Task 3 is wide but shallow** — mechanical `.into()` → `try_into()` churn
  across ~25 test sites. Right-sized as its own commit so tasks 4-8 review
  cleanly.
- **`InvalidSlug` (task 9)** may prove reachable. Confirm before deleting;
  AtomPub maps both variants to `BadRequest`.

---

## Global constraints

- **Rust, exact crates.** `common`, `storage`, `web`, `server`, `test-support`.
- **Backend parity.** Storage tests use the dual-backend template
  (`CONTRIBUTING.md`); a bare `#[tokio::test]` that should be dual-backend fails
  the `test-backend-pattern` guard. Never put tests in ADR-0019 per-backend
  dialect files.
- **Coverage policy** per ADR-0050. A large refactor shifts llvm-cov line
  attribution and can expose a pre-existing marker in a file you didn't touch —
  tell a latent failure from a new one before reaching for a suppression.
- **The gate.** `cargo xtask check` before every commit (`jaunder-commit`); the
  pre-commit hook runs it and will reformat, then fail the commit — re-stage and
  re-commit, that is normal. **No `Co-Authored-By` trailer.**
- **Staging.** The index arrives pre-populated. `git restore --staged .` then
  `git add -u` before each commit, or a partial commit silently sweeps in
  unrelated work.
- **`-p jaunder` in nextest.** `cargo nextest run -p web <name>` silently runs
  nothing for a `#[cfg(feature = "server")]` test — it prints `no tests to run`,
  which reads as success. Always include `-p jaunder`.

---

## Task 1 — ADR: `PostBody` carries a non-blank invariant

Uses **`jaunder-adr`** (numberless draft in `docs/adr/drafts/`;
`cargo xtask adr promote` numbers it at ship, after the final rebase).

**Files:**

- Create: `docs/adr/drafts/post-body-non-blank-invariant.md`
- Modify: `docs/adr/0063-domain-value-newtype-convention.md` — supersede note on
  the infallible-kind definition (§3, around 344-351)

**Where the language actually lives (verified 2026-08-05).** ADR-0063 does
**not** say "no length bound — any body is valid" — that sentence is a _rustdoc
line_ at `common/src/post_body.rs:5`, and rewriting it is part of task 2.
ADR-0063's only `PostBody` mention is 350-351, listing it as a first user of
`#[str_newtype(infallible)]`. So this task amends the _kind_ definition, not a
body-specific claim.

**Content:** a body with no non-blank line is not a body — any body _length_
remains valid, so this narrows the invariant without introducing a length bound.
State the one-door decision (validation applies to sqlx/serde decode too, no
trusted bypass) and the title-only-Org consequence, so task 6 is not later
mistaken for a regression.

Also record the **two-layer split** and why it is forced: `PostBody` has no
format, so the invariant (non-blank) is format-agnostic and lives in `from_str`,
while normalization is format-aware and lives in `canonicalize_body`. Include
the whitespace rule and its one accepted lossy case (unclosed code fence) from
the review header — an ADR is where a deliberate lossy normalization belongs, so
the next reader does not "fix" it.

**Coordinate with #830** (filed 2026-08-05 as the `PostTitle` sibling — same
shape, a type declared `infallible` that has an invariant). The maintainer asked
that the ADR-0063 correction land as **one coherent edit** to §3 rather than two
independent rewordings: the real defect is that §3's discriminator is written as
a test on the constructor's signature ("never rejects") while §2 frames it
invariant-first ("fallible when the value has an invariant"). Write the §3 edit
so it fixes that framing for both types, and keep the body-specific consequences
in the draft. **Do not** generalize decision 1 (no `from_trusted` bypass) into
the ADR: it holds here only because no blank-body rows exist, and #830 records
that blank _titles_ demonstrably did accumulate (migration
`0010_nullable_post_titles.sql`). That divergence must stay available.

**Verify:** `cargo xtask check` — `adr-format` passes. Do **not** run
`adr promote` yet.

- [ ] 1. ADR draft written

---

## Task 2 — `PostBody` gains a validating door

**Files:**

- Modify: `common/src/post_body.rs`
- Test: in-file `#[cfg(test)]` (crate convention)

**Interfaces:**

```rust
/// Error returned when a string cannot be parsed as a [`PostBody`].
#[derive(Debug, Error)]
#[error("post body must contain at least one non-blank line")]
pub struct InvalidPostBody;

impl FromStr for PostBody {
    type Err = InvalidPostBody;

    /// Validates, then stores **verbatim**. This door is format-agnostic, so it
    /// cannot normalize: whether trailing whitespace is content depends on the
    /// `PostFormat`, which lives beside the body. Normalization is
    /// [`canonicalize_body`]'s job. Unlike `PostSummary`, never trim here.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.split('\n').all(|line| line.trim().is_empty()) {
            return Err(InvalidPostBody);
        }
        Ok(PostBody(s.to_owned()))
    }
}
```

Drop `#[str_newtype(infallible)]` (line **49**) and the
`impl From<String> for PostBody` (**52-56**). **Also rewrite the type's rustdoc
at line 5** —
`/// An **infallible** wrapper (no length bound — any body is valid; see ADR-0063 and issue #402)`
is the sentence task 1 is amending, and it must stop claiming infallibility
here. The `StrNewtype` derive then routes serde **and** sqlx through `FromStr` —
that is what makes this one door rather than two, per issue decision 1.

**Red first (prove the guard bites):** write the rejection test against the
_pre-change_ type and watch it fail to compile / pass vacuously before
implementing.

```
cargo nextest run -p common post_body
```

Expected: FAIL before, PASS after.

**Tests:**

- `post_body_rejects_whitespace_only` — `""`, `"   "`, `"\n\n"`, `"  \n \t \n"`
  all `Err`.
- `post_body_accepts_body_with_blank_lines_around_content` — `"\n\n  hi  \n\n"`
  is `Ok`.
- `post_body_preserves_verbatim_after_validation` — the accepted value still
  compares equal to the original **untrimmed** string. Necessary but weak: it
  pins the wrapper, not the reason.
- **`markdown_body_with_leading_indent_still_renders_as_code_block`** — the
  load-bearing guard, and it belongs in `common/src/render.rs`, not
  `post_body.rs`. Body `"    fn main() {}\n"`, format `Markdown`, asserts the
  output contains `<pre><code>` and not `<p>fn main()`. Without this, a later
  "tidy-up" trim in the constructor passes every test in `post_body.rs` and
  silently turns users' code blocks into prose. Verified 2026-08-05 to
  discriminate: it fails on a trimmed body, passes on a verbatim one.
- Update `post_body_serde_round_trips_as_plain_string` (**79**) for the fallible
  deserialize, and add `post_body_deserialize_rejects_blank` — the wire door.
- The `compile_fail` doctests keep working; their `PostBody::from("x")` fixtures
  (lines **24, 37, 43**) are already valid values, but the _constructor_ changes
  — they become parses.

The existing in-file tests are exactly four:
`post_body_wraps_verbatim_without_trimming` (**63**),
`post_body_display_and_deref_expose_inner` (**71**),
`post_body_serde_round_trips_as_plain_string` (**79**),
`post_body_into_string_extracts_inner` (**89**).

- [ ] 2. `PostBody::from_str` validates; `infallible` removed; tests green

---

## Task 3 — repair every construction site

Mechanical, but it is what makes task 2 land. Separate commit so tasks 4-8
review cleanly.

**Files (production doors first — these are the real boundaries):**

- `server/src/atompub/mapping.rs:85` — `PostBody::from(value)` becomes a parse;
  map the error to `HandlerError::BadRequest` alongside the existing `EmptyPost`
  mapping at `server/src/atompub/mod.rs:249,263`.
- `web/src/posts/api.rs` — `PostInputs.body` is validated by the derived
  `Deserialize`; confirm the rejection surfaces as a 400 and not a 500, and that
  ADR-0065's arg-decode story still reads true.
- `storage/src/helpers.rs:302`, `storage/src/posts.rs` — the sqlx decode path
  now validates. No code change expected; confirm it compiles and that `PostRow`
  decode errors are typed sensibly.

**Files (tests/fixtures — `.into()` → a parse helper):**

- `common/src/render.rs` — **17** sites, 1383-1638 (1383, 1406, 1437, 1443,
  1475, 1490, 1500, 1515, 1535, 1544, 1554, 1562, 1573, 1579, 1587, 1599, 1638)
- `storage/src/posts.rs:3787,3814`
- `storage/src/test_support.rs:836,1357,1392`
- `storage/src/test_support.rs:850,961,1157` — **three `impl Into<PostBody>`
  builder setters**, missed by the original sweep. These are generic bounds, not
  call sites: when `From<&str>`/`From<String>` go away the bound stops being
  satisfiable by a `&str` and every caller breaks. Decide the shape here (take
  `PostBody` directly, or keep the bound and make callers parse) before churning
  the call sites above, or you will do task 3 twice.

**Not a site: `server/tests/helpers/mod.rs:436`.** That `PostBody` is a _local
test enum_ (`enum PostBody { Form(String), Json(String) }`, defined at line 408)
for HTTP request bodies — unrelated to `common::post_body::PostBody`, which that
file never references. The original plan listed it in error; it needs no change.

For fixtures, add a `parse_post_body` to `common::test_support` beside the
existing `parse_slug` (**376**) / `parse_username` (**364**), rather than
sprinkling `.unwrap()`. Nearest models to copy: `parse_post_title` (**245**),
`parse_post_summary` (**271**).

```
cargo xtask check --no-test
cargo nextest run -p common -p storage -p jaunder
```

Expected: PASS.

- [ ] 3. Tree compiles; all construction sites go through the validating door

---

## Task 4 — `canonicalize_org_body` becomes a typed fallible seam

**Files:**

- Modify: `common/src/render.rs`
- Modify: `storage/src/post_service.rs` (call sites 316, 483)

**Interfaces:**

```rust
/// Canonicalize an ingested Org body (ADR-0024) …
///
/// # Errors
///
/// Returns `InvalidPostBody` when canonicalization consumes the whole body — a
/// title-only post. See #811 decision 2.
pub fn canonicalize_org_body(body: &PostBody) -> Result<PostBody, InvalidPostBody>
```

The existing `canon_*` unit tests keep their inputs but assert through the new
type. `canon_is_idempotent` matters most — it must still hold, and now also
proves the `Result` is `Ok` on a second pass.

**Scope note:** this task is the _typing_ change only — same algorithm, new
signature. The whitespace-rule change (the terminating-`\n` restore) lands in
task 5, where it applies to both formats at once. Keeping them apart means task
4's diff is pure plumbing and any `canon_*` expectation that moves in task 5 is
visibly a behaviour decision.

```
cargo nextest run -p common canon
```

Expected: PASS.

- [ ] 4. `canonicalize_org_body` takes and returns `PostBody`, fallible

---

## Task 5 — the Markdown seam; `post_service` stops special-casing Org

**Files:**

- Modify: `common/src/render.rs`
- Modify: `storage/src/post_service.rs`

Replace the two inline `if matches!(format, PostFormat::Org)` blocks
(`post_service.rs:316,483`) with one format-dispatched call, so both formats go
through the same door and neither is privileged:

```rust
/// The stored form of an authored body.
///
/// Markdown and Org share a whitespace normalization; Org additionally has its
/// title-source line stripped (ADR-0024); HTML is verbatim passthrough and is
/// deliberately exempt. One seam, so a format is added by extending this match,
/// not by editing two call sites in `storage`.
///
/// # Errors
///
/// Returns `InvalidPostBody` when normalization consumes the whole body — a
/// title-only Org post. See #811 decision 2.
pub fn canonicalize_body(
    body: &PostBody,
    format: &PostFormat,
) -> Result<PostBody, InvalidPostBody>
```

**The normalization, exactly** (decided 2026-08-05; see the review header for
the measurements):

```
Html            => verbatim, unchanged
Markdown | Org  => 1. drop leading all-whitespace lines
                   2. trim_end()
                   3. re-append one '\n' if non-empty
Org             => additionally strip the title source (#+TITLE: / leading `* `)
empty result    => Err(InvalidPostBody)
```

Never strip leading _horizontal_ whitespace from a line with content (that is
the indented-code-block case), and never touch interior blank lines
(loose-vs-tight lists). Step 3 is not optional — bare `trim_end()` eats the
terminating newline, which is significant inside `<pre><code>` and Org
paragraphs.

`canonicalize_org_body` already implements steps 1-2 plus the title strip
(`render.rs:723-767`, verified equal on 9 inputs), so this is mostly a
generalization, not a new algorithm. Step 3 is genuinely new and **changes Org's
current output** — existing `canon_*` expectations that end without a newline
will need updating; that is the fix, not a regression.

**Tests:**

- `PostFormat`-parameterized: every variant round-trips a normal body unchanged
  except Org's title line. Keep the storage-level
  `test_perform_post_creation_markdown_body_is_not_canonicalized` — but **rename
  it**, because after this task Markdown _is_ canonicalized (whitespace-only);
  what it still pins is that Markdown's _content_ is untouched.
- **`canonicalize_preserves_indented_code_block`** — `"\n\n    fn main() {}\n"`,
  `Markdown`: renders `<pre><code>`, not `<p>`. The guard for step 1.
- **`canonicalize_preserves_terminating_newline`** — `"    code\n"`, `Markdown`:
  rendered output retains `code\n` inside `<pre><code>`. The guard for step 3.
- **`canonicalize_preserves_interior_blank_lines`** — `"- a\n\n- b\n"` still
  renders a _loose_ list (`<li><p>a</p></li>`). The guard for the interior rule.
- **`canonicalize_leaves_html_verbatim`** — an HTML body with leading and
  trailing whitespace is returned byte-identical.
- **`canonicalize_is_idempotent` for every format** — `f(f(x)) == f(x)`. This is
  what makes the sqlx decode door safe.
- **`canonicalize_truncates_trailing_blanks_in_unclosed_fence`** — the _accepted
  lossy_ case, pinned deliberately so it reads as a decision, not a bug:
  `"```\ncode\n\n"` loses the blank line inside the unclosed fence. Name it so
  the next reader sees intent; reference the ADR from task 1.

```
cargo nextest run -p common canonicalize
cargo nextest run -p common canon
cargo nextest run -p storage -p jaunder post_service
```

Expected: PASS.

- [ ] 5. One canonicalization seam over both formats

---

## Task 6 — reject the title-only Org post

The behaviour change. Its own commit so it is visible in review and revertable
alone.

**Files:**

- Test: `storage/src/post_service.rs` (in-file, dual-backend template)
- Test: `server/tests/atompub/atompub_posts.rs` — the AtomPub boundary returns
  400

**Tests:**

- `perform_post_creation_rejects_title_only_org_body` — body `"* My Title\n"`,
  format `Org`. Today: succeeds with an empty stored body. After: rejected.
- The AtomPub twin, asserting `400` and not `500`.
- `end2end/tests/posts.ts` — **already checked (2026-08-05): there is no Org
  fixture at all.** `createPost` hardcodes `format: "markdown"` (line 36) and
  the UI helper (58-62) fills only body/summary/slug. So e2e does not exercise
  this change and needs no edit — that is the finding, not a skipped step. Worth
  one line in the PR body: the user-visible behaviour change has no e2e
  coverage, by absence of any Org path in e2e.

**Prove it bites:** run against the pre-change tree and watch it fail.

```
cargo nextest run -p storage -p jaunder title_only
```

Expected: FAIL before task 4/5, PASS after.

- [ ] 6. Title-only Org body rejected, at the service and the AtomPub boundary

---

## Task 7 — `derive_post_title` goes total

**Files:**

- Modify: `common/src/render.rs`
- Modify: `storage/src/post_service.rs`

**Interfaces:**

```rust
/// Derives a post's public title and its slug.
///
/// Total: an empty post is unrepresentable ([`PostBody`]), so there is no
/// nothing-to-store case left to report. Renamed — it has never derived only a title.
pub fn derive_post_naming(
    explicit_title: Option<&str>,
    body: &PostBody,
    format: &PostFormat,
) -> (Option<PostTitle>, Slug)
```

It performs the `slugify_title` itself; both call sites currently do that
immediately after, so the `String` seed stops existing. `PostTitle` and `Slug`
are distinct types, so the positional pair is no longer transposable — which is
why this needs no wrapper struct.

**Then retire `EmptyPost`:** remove `PerformCreationError::EmptyPost` and
`PerformUpdateError::EmptyPost` and their `.ok_or(…)` at
`post_service.rs:311,478`. Retarget the existing tests —
`test_perform_post_creation_empty_body` (658) becomes a `PostBody` parse test
from task 2, and the `Debug`/`From` tests at 1263-1382 lose their variants.
Check `server/src/atompub/mod.rs:441,461` and its `status()` assertions.

```
cargo nextest run -p common -p storage -p jaunder
```

Expected: PASS.

- [ ] 7. `derive_post_naming` total, returns `(Option<PostTitle>, Slug)`;
     `EmptyPost` gone

---

## Task 8 — delete `update_rendered_post`

**Files:**

- Modify: `storage/src/post_service.rs` — delete `update_rendered_post`
  (149-183) and `RenderedPostUpdate` (125-147). Both ranges re-verified exact.
- **Not `storage/src/lib.rs`** — there is no named re-export to drop; line 60 is
  a blanket `pub use post_service::*;`. Deleting the items suffices.
- Modify: `server/tests/storage/mod.rs` — the three tests at 4830, 4867, 4901
  (all exact), plus the section banner comment at **4609** and the now-dead
  imports at **line 20** (`update_rendered_post`) and **line 24**
  (`RenderedPostUpdate`).

It is a byte-identical clone of `perform_post_update`'s tail (331-347 vs
169-182, differing only in the error map) with no production callers.

**For each test, decide and record which:**

- `update_rendered_post_markdown_renders_and_updates` /
  `..._org_renders_and_updates` — these pin real behaviour (render-on-update,
  Org canonicalization on update). Retarget at `perform_post_update`, or delete
  if `test_perform_post_update_canonicalizes_org_body` (971) already covers it.
  Check before deleting; do not assume.
- `update_rendered_post_not_found_returns_storage_error` — likely already
  covered by a `perform_post_update` not-found test. Confirm, then delete.

Four test deletions were authorized in #569 by name; **these are not those**.
Anything deleted here needs its coverage shown to exist elsewhere, in the commit
message.

```
rg 'update_rendered_post|RenderedPostUpdate'   # expect no hits
cargo nextest run -p storage -p jaunder
```

Expected: no hits; PASS.

- [ ] 8. `update_rendered_post` and `RenderedPostUpdate` deleted; tests resolved

---

## Task 9 — confirm or remove `InvalidSlug` on the derived path

`slugify_title` never fails (falls back to `"post"`) and its output is
documented idempotent through `Slug::from_str`, so
`.parse::<Slug>().map_err(|_| …InvalidSlug)` looks unreachable once task 7 folds
the slugify in.

**Prove it before deleting.** Temporarily `unreachable!()` the arm and run the
full suite; if nothing trips it, remove the variant. If something does, leave it
and note why in the commit message.

Check `server/src/atompub/mod.rs:249,263` — both map `InvalidSlug` to
`BadRequest`; removing the variant touches that match, and the paired `status()`
assertions at **445** and **465** go with it.

`candidate_slug` keeps its `Result` — its own doc says the suffixed candidate is
a different question from the seed.

```
cargo nextest run -p storage -p jaunder
cargo xtask check
```

- [ ] 9. `InvalidSlug` proven reachable (kept, documented) or proven dead
     (removed)

---

## Task 10 — close out

- Close #785 with a pointer: no bare-`String` slug seed survives task 7, so
  there is nothing left to newtype. Use **`jaunder-issues`**.
- `rg 'slug_seed'` — **expected landing state, not "no hits".** There are 21
  code hits today: `common/src/render.rs` ×12 (all tests: 896, 903, 908, 915,
  921, 924, 930, 933, 938, 945, 1024, 1027) and `storage/src/post_service.rs` ×9
  (310, 326, 410, 412, 418, 477, 489, 495, 502). Of those, **410-418 are
  `candidate_slug`, whose parameter is already a typed `&Slug`** and stays. Only
  the bare-`String` seeds at 310, 326, 477, 495 disappear under task 7. So the
  pass condition is "every surviving `slug_seed` is a `Slug`", which is what
  #785 asked for.
- Confirm #797 is untouched and still accurate (it is not blocked by this). For
  reference, `parse_post_cursor` has 5 code hits — `storage/src/posts.rs:438`
  (the fn) and 3671, 3679, 3680, 3686 — plus a comment at
  `server/tests/web/web_posts.rs:1299`.
- `cargo xtask adr promote` — **after** any final rebase, so the number cannot
  collide.
- Full gate.

```
cargo xtask validate
```

Expected: PASS, including the four-combo e2e matrix.

- [ ] 10. Follow-ups closed, ADR promoted, `validate` green
