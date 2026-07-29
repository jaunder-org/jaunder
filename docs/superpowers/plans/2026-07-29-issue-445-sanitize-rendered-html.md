# Issue #445 — Sanitize rendered HTML: implementation plan

> **For agentic workers:** Execute task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Tick each `- [ ]` in real time.

**Spec:**
[`docs/superpowers/specs/2026-07-29-issue-445-sanitize-rendered-html.md`](../specs/2026-07-29-issue-445-sanitize-rendered-html.md)
— the "what/why". This plan is the "how" and does not restate it.

## Goal

Move the safety guarantee off `render()` and onto the `RenderedHtml` type: one
host-only door that **establishes** safety by sanitizing, one that **inherits**
it for our own round-trips, and an existing xtask gate that makes any third path
fail the build.

## Scope

**In:** `ammonia` + a `sanitize` feature on `common`; `RenderedHtml::sanitize`;
`render()` routed through it and gated behind the feature; a `sqlx::Decode` impl
and the `PostRow`/`build_post_record` change it enables; the gate's allowlist
shrinking to one door plus a negative test; three stale doc comments; an ADR
draft.

**Out:** #282's ingestion path (spec D9 — addendum already filed); backfill
(D7); sanitizing on read (D3); removing `pulldown-cmark`/`orgize` from the wasm
bundle.

## Task list

1. Add `ammonia` and the `sanitize` feature; prove the wasm graph stays clean.
2. Add `RenderedHtml::sanitize` and pin the allowlist with tests.
3. Route `render()` through `sanitize` and gate it behind the feature.
4. Add `sqlx::Decode`; decode `rendered_html` directly into the newtype.
5. Tighten the gate to one door and prove it still bites.
6. End-to-end regression test through the real storage path.
7. Draft the ADR.
8. Final verification.

## Key risks/decisions

- **Cold rebuild.** `ammonia` is not vendored; Task 1 front-loads that cost so
  it is not a mid-run surprise.
- **Feature unification is load-bearing.** Coverage runs bare
  `cargo llvm-cov --no-report nextest` with no `--features`. `common`'s gated
  code compiles in the host suite only because `storage` requests
  `common/sanitize` and the workspace build unifies. The CSR build
  (`-p web -p client -p csr --features csr`) excludes `storage`, so `ammonia`
  stays out of wasm. Task 1 verifies both directions rather than assuming.
- **Over-aggressive stripping.** The allowlist must not eat what our renderers
  emit; Task 2 pins that with tests against real `pulldown-cmark`/`orgize`
  output.
- **`Decode` blesses any text column** typed as `RenderedHtml` — accepted
  residual risk (spec D3). Task 4 rewrites the rationale comment to record the
  reconsideration.

**Tech stack:** Rust, `ammonia`, `sqlx`, `cargo nextest`, `cargo xtask`, `syn`
(the gate).

## Global Constraints

- Work in `.claude/worktrees/issue-445-sanitize-rendered-html` on branch
  `worktree-issue-445-sanitize-rendered-html`.
- Use `devtool run -- <cmd>` for every build/test/gate command.
- Storage tests follow the dual-backend template (`CONTRIBUTING.md` "backend
  parity"); a bare `#[tokio::test]` that should be dual-backend fails the
  `test-backend-pattern` guard.
- No `#[allow(...)]` / `#[expect(...)]` additions without explicit user
  approval.
- `RenderedHtml`'s tuple field stays private; no new public constructor beyond
  `sanitize`.
- Before each commit run `devtool run -- cargo xtask check`; commit via
  `jaunder-commit`. **No `Co-Authored-By` trailer.**

---

### Task 1: Add `ammonia` and the `sanitize` feature

**Files:** `Cargo.toml` (workspace), `common/Cargo.toml`, `storage/Cargo.toml`

**Interfaces:** Produces a `sanitize` feature on `common` that pulls `ammonia`;
off by default, enabled by `storage`.

- [x] **Step 1: Declare the dependency.** (`ammonia = "4"`; registry has 4.1.4)

Workspace `Cargo.toml` `[workspace.dependencies]`: `ammonia = "4"`. Verify the
resolved major version rather than trusting this literal — if 4.x is not
current, use what resolves and note it.

`common/Cargo.toml`:

```toml
ammonia = { workspace = true, optional = true }
```

and under `[features]`:

```toml
# Optional, host-only HTML sanitizer backing `RenderedHtml::sanitize`. Off by
# default and never enabled for wasm — the CSR/wasm build pulls `common` without
# it, so `ammonia` is never in the wasm dependency graph. Deliberately its own
# feature rather than riding `sqlx`: rendering is not persistence.
sanitize = ["dep:ammonia"]
```

`storage/Cargo.toml`: add `"sanitize"` to `common`'s feature list, both the main
dependency and the `[dev-dependencies]` entry if it names features.

- [x] **Step 2: Build and pay the cold rebuild.**

Run: `devtool run -- cargo build -p common --features sanitize`

Expected: PASS. First run performs a full cold vendor rebuild — allow time.

- [x] **Step 3: Prove the wasm graph is clean.** (0 matches for
      `ammonia|html5ever`)

Run: `devtool run -- cargo tree -p csr --target wasm32-unknown-unknown`

Then grep the parked output for `ammonia`. Expected: **no match** — this is AC7.

- [x] **Step 4: Prove the host graph has it.** (2 matches)

Run: `devtool run -- cargo tree -p storage`

Grep the parked output for `ammonia`. Expected: **match** — confirms unification
delivers the feature to `common` in the host suite.

- [x] **Step 5: Gate and commit.**

Run: `devtool run -- cargo xtask check`. Commit:
`build(common): add ammonia behind a host-only sanitize feature (#445)`.

---

### Task 2: Add `RenderedHtml::sanitize` and pin the allowlist

**Files:** `common/src/render.rs`

**Interfaces:** Produces
`#[cfg(feature = "sanitize")] pub fn RenderedHtml::sanitize(raw: &str) -> Self`
— the only public door that mints from outside data. Field stays private.

- [x] **Step 1: Write the tests first.**

In `common/src/render.rs`'s `#[cfg(test)] mod tests`, gated
`#[cfg(feature = "sanitize")]`, add:

- `sanitize_strips_script_element` — `<script>alert(1)</script>` does not
  survive.
- `sanitize_strips_event_handler_attributes` — `<img src=x onerror=alert(1)>`
  keeps no `onerror`.
- `sanitize_strips_javascript_urls` — `<a href="javascript:alert(1)">x</a>`
  keeps no `javascript:`.
- `sanitize_preserves_formatting_markup` — emphasis, headings, lists, tables,
  and a fenced code block (`<pre><code class="language-rust">`) survive intact.
- `sanitize_preserves_safe_links` — `<a href="https://example.com">` survives
  with its `href`.

Assert on absence of the dangerous token (`contains("<script")`,
`contains("onerror")`, `contains("javascript:")`), not on exact output —
ammonia's escaping details are not our contract.

- [x] **Step 2: Run the tests, verify they fail.**

Run: `devtool run -- cargo nextest run -p common sanitize_`

Expected: FAIL — `RenderedHtml::sanitize` is not defined. (Confirmed: 5× E0599.
Note the run needs `--features sanitize`, since the tests and the door are both
gated on it.)

- [x] **Step 3: Implement the door.**

```rust
/// Sanitize untrusted HTML into a `RenderedHtml`. This is the door for anything
/// originating **outside** jaunder — an authored post body's rendered output, an
/// ingested feed entry (#282), any future inbound producer. It *establishes* the
/// type's invariant (no active markup) rather than asserting it, which is what
/// distinguishes it from [`RenderedHtml::from_trusted`].
#[cfg(feature = "sanitize")]
#[must_use]
pub fn sanitize(raw: &str) -> Self {
    Self(ammonia::clean(raw))
}
```

Start from `ammonia::clean` (the audited default allowlist, spec D6). Only if
Step 4 shows it stripping legitimate renderer output, switch to an
`ammonia::Builder` configured once as a module-level constant — one allowlist,
defined in one place.

- [x] **Step 4: Run the tests, verify they pass.**

**Allowlist finding — one deliberate widening, via a `Builder`.**
`ammonia::clean`'s default preserves everything our renderers emit (headings,
`em`/`strong`, lists, `pre`/`code`, tables incl. `thead`/`th`/`td`, blockquotes,
safe `a`/`img`). The one thing it drops is `class` on `<code>`, which loses
`pulldown-cmark`'s `language-rust` fence marker — so the default is **not**
sufficient after all, and a `SANITIZER` `Builder` (a module-level `LazyLock`,
one policy shared by every caller) re-admits it.

Re-admitting `class` via the tag/attribute allowlist alone would permit _any_
class on attacker-supplied content, letting a post borrow the app's own CSS to
mimic or hide UI. So an `attribute_filter` narrows the surviving values to
`language-*` tokens on `pre`/`code` only. Both halves are pinned by tests: the
language marker survives, `j-anon-only` alongside it does not, and `class` on a
`<p>` is still dropped entirely.

Run: `devtool run -- cargo nextest run -p common sanitize_`

Expected: PASS. If `sanitize_preserves_formatting_markup` fails, the default
allowlist is too tight for our renderers — widen via `Builder` and record what
was added and why in a comment.

- [x] **Step 5: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`feat(common): add RenderedHtml::sanitize, the establishing door (#445)`.

---

### Task 3: Route `render()` through `sanitize`

**Files:** `common/src/render.rs`, `storage/src/post_service.rs`,
`storage/src/test_support.rs`

**Interfaces:** `render()` becomes `#[cfg(feature = "sanitize")]` and returns
sanitized output. Signature is otherwise unchanged — no new parameter.

- [x] **Step 1: Write the format-coverage tests first (AC1).**

In `common/src/render.rs` tests, gated `#[cfg(feature = "sanitize")]`:

- `render_markdown_strips_embedded_script`
- `render_org_strips_embedded_script`
- `render_html_passthrough_is_sanitized` — the `PostFormat::Html` raw path (spec
  D5).

Each renders a body carrying `<script>alert(1)</script>` and asserts it does not
survive.

- [x] **Step 2: Run them, verify they fail.** (3/3 red — the hole was real in
      every format.)

Run: `devtool run -- cargo nextest run -p common render_`

Expected: FAIL on the three new tests — current `render()` passes markup
through.

- [x] **Step 3: Gate `render()` and route it.**

**Gating cascades.** `render()` is the only caller of
`render_markdown`/`render_org` and the only user of the `PostBody` import, so
all three had to be gated too or the wasm build fails `-D warnings` with
dead-code/unused-import errors. Also updated `RenderedHtml`'s **type-level** doc
— a fourth stale comment the plan didn't list, and the clearest statement of the
old provenance-not-safety model in the tree.

Add `#[cfg(feature = "sanitize")]` to `render()`. Change its tail from
`RenderedHtml(html)` to `Self::sanitize(&html)` — i.e.
`RenderedHtml::sanitize(&html)` — so there is exactly one sanitizing path.

Note in its doc comment that it is host-only and why: with the feature off the
function does not exist, rather than existing and silently not sanitizing.

Leave `derive_post_metadata` ungated — it uses the parsers but not `ammonia`.

- [x] **Step 4: Run the full common suite.** (444/444.)

Two pre-existing tests needed adjusting, both genuine consequences rather than
cosmetic fixes: `render_html_format_is_identity` became
`render_html_format_preserves_safe_markup`, since `Html` is no longer a verbatim
passthrough; and the Org case had to switch from `#+begin_export html` to the
inline `@@html:…@@` form, because orgize escapes the former itself — the first
draft of that test was asserting on escaped, harmless text.

Run: `devtool run -- cargo nextest run -p common`

Expected: PASS. Pre-existing `render` tests that assert exact HTML may now see
sanitized output; if any fail, confirm the change is sanitization (not
corruption) before adjusting the expectation.

- [x] **Step 5: Confirm call sites still build.** (Unchanged, as predicted — the
      signature did not change.) Note bare `cargo nextest run -p storage`
      reports false `case_2_postgres` ConnectionRefused failures; only the xtask
      gate provides the ephemeral Postgres.

`render()`'s three call sites (`post_service.rs:79`, `:169`, `:331`) and two in
`test_support.rs` (`:1017`, `:1373`) need no change — the signature is unchanged
and `storage` enables the feature.

Run: `devtool run -- cargo nextest run -p storage`

Expected: PASS.

- [x] **Step 6: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`fix(common): sanitize rendered post HTML in all three formats (#445)`.

---

### Task 4: Add `sqlx::Decode` and decode into the newtype

**Files:** `common/src/render.rs`, `storage/src/helpers.rs`,
`storage/src/posts.rs`

**Interfaces:** Produces `impl sqlx::Decode for RenderedHtml` under the existing
`sqlx` feature, constructing the private field directly — it uses neither
`sanitize` nor `from_trusted`.

- [x] **Step 1: Rewrite the rationale comment.**

`common/src/render.rs:185-191` currently explains why there is **no** `Decode`.
Replace it with why there now **is** one, and what that costs: a decode blesses
any text column typed as `RenderedHtml`, accepted because writes sanitize and
the gate enforces it (spec D3). Record that a _sanitizing_ decode was considered
and rejected — no production data to heal, and it would cost an html5ever parse
per post per read forever.

- [x] **Step 2: Implement `Decode` inside the existing
      `#[cfg(feature = "sqlx")]` block.**

```rust
impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for RenderedHtml
where
    String: sqlx::Decode<'r, DB>,
{
    fn decode(value: <DB as sqlx::Database>::ValueRef<'r>)
        -> Result<Self, sqlx::error::BoxDynError>
    {
        <String as sqlx::Decode<DB>>::decode(value).map(Self)
    }
}
```

`Self(..)` is the private constructor — legal here because this is `common`.

- [x] **Step 3: Decode the column directly.**

Also had to add `Type::compatible`, delegating to `String`'s like every other
newtype bridge. It was previously omitted _because_ there was no decode path;
the trait default accepts only the exact `type_info`, which would reject an
equally-valid `VARCHAR` column. And five `#[cfg(test)]` `PostRow` fixtures built
the field from a raw `String` — now `from_trusted`, which is gate-exempt in
tests.

`storage/src/helpers.rs`: change `PostRow.rendered_html` from `String` to
`RenderedHtml` (joining every other domain column). In `build_post_record`,
replace `rendered_html: RenderedHtml::from_trusted(row.rendered_html)` with
`rendered_html: row.rendered_html`, and update the comment above it — the
"trusted rebuild" note no longer describes what happens.

- [x] **Step 4: Correct the aspirational storage comments.**

The issue's quoted wording ("Sanitized HTML rendering") no longer exists
anywhere — it had already been reworded. The current form was two
`PostRecord`/revision field docs reading "A provenance marker, **not** a safety
guarantee — `render()` does not sanitize (see #445)", which this change makes
false. Both now state the guarantee. A tree-wide sweep for
`provenance marker|does not sanitize` returns nothing.

`storage/src/posts.rs`: the doc comments calling the column "Sanitized HTML
rendering" are now true. Reword so they state the guarantee and where it comes
from (`RenderedHtml::sanitize`) rather than reading as an unbacked label.

- [x] **Step 5: Run the storage suite.** (Via the gate, which supplies
      Postgres.)

Run: `devtool run -- cargo nextest run -p storage`

Expected: PASS. A decode-type mismatch surfaces here as a column-decode error.

- [x] **Step 6: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`refactor(storage): decode rendered_html directly into RenderedHtml (#445)`.

---

### Task 5: Tighten the gate to one door

**Files:** `xtask/src/steps/rendered_html_from_trusted_check.rs`,
`common/src/render.rs`

**Interfaces:** `ALLOWED_FNS` reduces to `["deserialize_rendered_html"]`; the
recovery message names `RenderedHtml::sanitize`.

- [x] **Step 1: Write the negative test first.**

In the check's `#[cfg(test)] mod tests`, add
`an_inbound_shaped_fn_using_from_trusted_is_flagged`:

```rust
let src = "\
fn ingest_feed_entry(remote_html: String) -> RenderedHtml {
    RenderedHtml::from_trusted(remote_html)
}
";
assert!(!violations(src).unwrap().is_empty());
```

This is AC4 — the gate must demonstrably bite on the #282-shaped mistake.

- [x] **Step 2: Run it, verify it passes already.** (It did — the gate needed no
      logic change to catch the #282 shape.)

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml an_inbound_shaped_fn`

Expected: PASS — the gate already flags non-allowlisted callers. The test pins
that behavior against future regression rather than driving new code.

- [x] **Step 3: Shrink the allowlist.**

**Three of the gate's own tests used `build_post_record` as their allowlisted
example and had to be repointed at `deserialize_rendered_html`.** The important
one is `a_nested_fn_shadowing_an_allowed_name_is_still_flagged`: with the name
no longer allowlisted it would still have _passed_, but vacuously — flagged for
the ordinary reason rather than proving nesting cannot borrow an exemption. Left
unrepointed, the suite would have kept a green test that no longer tested
anything.

Remove `"build_post_record"` — after Task 4 it no longer calls `from_trusted`.
Keep `"deserialize_rendered_html"`.

Fix the stale location comment while here: it says `web/src/posts/mod.rs`, but
`deserialize_rendered_html` lives in `common/src/render.rs` and is used by
`common/src/seed.rs`.

- [x] **Step 4: Point the recovery message at the new door.** (Also rewrote the
      module header, which described the type as a provenance newtype with
      `render()` as its only mint.)

The message currently ends "otherwise obtain the `RenderedHtml` from `render()`
instead." Reword: data from **outside** must go through
`RenderedHtml::sanitize`; `from_trusted` is only for our own prior output
round-tripping.

Update the assertion in `problems_reports_file_line_and_recovery` if it pins
wording.

- [x] **Step 5: Rewrite `from_trusted`'s doc comment.**

`common/src/render.rs:99-102` predates any sanitizer. State that it _inherits_
safety from our own prior `sanitize` output round-tripping through our store or
wire, and contrast it with `sanitize`, which _establishes_ safety. Note it is
the sole remaining door and that the gate enforces this.

- [x] **Step 6: Run the gate against the real tree.**

Run: `devtool run -- cargo xtask check`

Expected: PASS, including `rendered-html-from-trusted`. A failure here means a
`from_trusted` call survives somewhere Task 4 was meant to remove it.

- [x] **Step 7: Commit.**

`refactor(xtask): reduce the from_trusted allowlist to one door (#445)`.

---

### Task 6: End-to-end regression test

**Files:** `storage/src/post_service.rs` (in-file `#[cfg(test)]`) or the storage
integration suite, following the crate's existing convention.

**Interfaces:** Consumes `storage::perform_post_creation`; asserts the persisted
`rendered_html` carries no active markup. This is AC2.

- [x] **Step 1: Write the test.** In `storage/src/post_service.rs`'s in-file
      `#[cfg(test)]`, reusing the existing `creation_with_key` fixture and the
      `#[apply(backends)]` dual-backend template.

Create a post whose body contains `<script>alert(1)</script>` and an
`<img src=x onerror=alert(1)>` via the real creation path, read the record back,
and assert the stored `rendered_html` contains neither `<script` nor `onerror`.

**Follow the dual-backend template** (`CONTRIBUTING.md` "backend parity") — a
bare `#[tokio::test]` here fails the `test-backend-pattern` guard.

Prefer an existing fixture from `storage::test_support` over hand-rolling setup.

- [x] **Step 2: Run it.** Green in the gate (both backends), and separately
      confirmed the `case_1_sqlite` case actually executes — "1 test run: 1
      passed" — rather than being silently absent.

Run: `devtool run -- cargo nextest run -p storage rendered_html`

Expected: PASS on both backends. If it fails, the creation path is not reaching
the sanitizing `render()`.

- [x] **Step 3: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`test(storage): assert malicious post bodies are sanitized end-to-end (#445)`.

---

### Task 7: Draft the ADR

**Files:** `docs/adr/drafts/rendered-html-sanitization.md` (numberless —
`cargo xtask adr promote` numbers it at ship)

- [x] **Step 1: Write the draft.** At
      `docs/adr/drafts/rendered-html-sanitization.md`, `Status: accepted`.

Follow `docs/adr/template.md`. Record:

- **Context:** `render()` emitted unsanitized HTML into an unescaped sink; #398
  gave provenance, not safety; `render()` is not the only inbound door (#282,
  #6).
- **Decision:** the guarantee lives on `RenderedHtml`, via two doors —
  `sanitize` establishes, `from_trusted` inherits — with the #398 gate making
  any third path fail the build. `ammonia` behind a host-only `sanitize`
  feature; no sanitizing on read.
- **Consequences:** `RenderedHtml`'s meaning shifts from provenance to invariant
  (amends #398's framing); `common` takes a second deliberate carve-out; a
  `Decode` blesses any column typed as `RenderedHtml`; the invariant is enforced
  host-side only, which is complete because all outside input arrives host-side.
- **Rejected:** trait-injected sanitizer (permits a no-op, weakening the
  guarantee it exists to provide); moving `render()` to `host`/`storage`;
  sanitize-on-read.

- [x] **Step 2: Format and commit.** — **correction: there is nothing to
      commit.**

`docs/adr/drafts/` is gitignored by design (ADR-0048/#219): a draft stays
numberless and out of git so its first appearance in history is already
collision-free. `cargo xtask adr promote` assigns the number, moves the file,
and stages it during **ship**, after the final rebase — so the ADR lands in the
ship commit, not here. Prettier was still run on the draft so promotion yields a
gate-clean file.

---

### Task 8: Final verification

- [x] **Step 1: Re-verify the wasm graph.** (Still 0 matches — AC7.)

Run: `devtool run -- cargo tree -p csr --target wasm32-unknown-unknown`; grep
for `ammonia`. Expected: no match (AC7).

- [x] **Step 2: Confirm one production `from_trusted` door remains.**

Confirmed: every remaining `RenderedHtml::from_trusted` call is a `#[cfg(test)]`
fixture except `deserialize_rendered_html`. (`ContentType::from_trusted` is a
separate, gate-exempt door.) The sweep also turned up a **fifth** stale comment
the plan hadn't listed — `PostRow`'s doc still described the write-only bridge
and the `from_trusted` rebuild that Task 4 removed. Fixed.

Grep `from_trusted` across `common/src`, `storage/src`, `server/src`, `web/src`,
excluding `#[cfg(test)]` sites. Expected: only `deserialize_rendered_html` (AC5,
AC4).

- [x] **Step 3: Full local gate.** — run **split**, both green, after a rebase
      onto `origin/main` (the branch was 7 commits behind, including #520's
      xtask gate changes; gating against the stale base would have proved
      nothing about what CI sees).

`devtool run -- cargo xtask validate --no-e2e` → PASS (481s), then
`devtool run -- cargo xtask e2e sqlite chromium` → PASS. The split was
pre-emptive rather than a fallback from an exit-124: eleven worktrees are live
on this host, which is exactly the contention that starves the four concurrent
e2e VMs. The remaining three combos (`sqlite×firefox`,
`postgres×{chromium, firefox}`) are left to CI's matrix, which is authoritative
per ADR-0034.

Post-rebase check for the silent-duplicate hazard (a clean auto-merge can double
an append-list entry): `ammonia` appears exactly once in each of the workspace,
`common`, and `storage` manifests, and all 8 commits replayed.

- [x] **Step 4: Conformance check.** All ten met; one test carries a different
      name than the plan predicted.

| AC   | Verdict | Evidence                                                                                                                           |
| ---- | ------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| AC1  | ✅      | `render_markdown_strips_embedded_script`, `render_org_strips_embedded_script`, `render_html_strips_embedded_script`                |
| AC2  | ✅      | `perform_post_creation_sanitizes_stored_rendered_html`, dual-backend, asserts on the **stored** column                             |
| AC3  | ✅      | `pub struct RenderedHtml(String)` — field private; `sanitize` the only door minting from outside data                              |
| AC4  | ✅      | `an_inbound_shaped_fn_using_from_trusted_is_flagged`; `ALLOWED_FNS == ["deserialize_rendered_html"]`                               |
| AC5  | ✅      | `PostRow.rendered_html: RenderedHtml`; every remaining `from_trusted` in `helpers.rs` is a `#[cfg(test)]` fixture                  |
| AC6  | ✅      | `sanitize_preserves_formatting_markup`, `sanitize_keeps_only_language_classes_on_code`, `sanitize_preserves_safe_links_and_images` |
| AC7  | ✅      | `cargo tree -p csr --target wasm32-unknown-unknown` — 0 matches for `ammonia\|html5ever`, **re-run after the rebase**              |
| AC8  | ✅      | `from_trusted` doc, the `Decode` rationale, and both `posts.rs` field docs now state the guarantee                                 |
| AC9  | ✅      | `docs/adr/drafts/rendered-html-sanitization.md` (numberless, gitignored)                                                           |
| AC10 | ✅      | see Step 3                                                                                                                         |

**AC1 naming note:** the third test is `render_html_strips_embedded_script`, not
the plan's predicted `render_html_passthrough_is_sanitized` — same body, same
coverage, named to match its two siblings. Recorded rather than renamed.

- [ ] **Step 5: Handoff.**

Invoke `jaunder-ship`: whole-branch review, archive spec + plan, push, PR,
monitor CI, halt before merge. This is security-adjacent, so it gets a full
review regardless of diff size.
