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

- [ ] **Step 1: Write the format-coverage tests first (AC1).**

In `common/src/render.rs` tests, gated `#[cfg(feature = "sanitize")]`:

- `render_markdown_strips_embedded_script`
- `render_org_strips_embedded_script`
- `render_html_passthrough_is_sanitized` — the `PostFormat::Html` raw path (spec
  D5).

Each renders a body carrying `<script>alert(1)</script>` and asserts it does not
survive.

- [ ] **Step 2: Run them, verify they fail.**

Run: `devtool run -- cargo nextest run -p common render_`

Expected: FAIL on the three new tests — current `render()` passes markup
through.

- [ ] **Step 3: Gate `render()` and route it.**

Add `#[cfg(feature = "sanitize")]` to `render()`. Change its tail from
`RenderedHtml(html)` to `Self::sanitize(&html)` — i.e.
`RenderedHtml::sanitize(&html)` — so there is exactly one sanitizing path.

Note in its doc comment that it is host-only and why: with the feature off the
function does not exist, rather than existing and silently not sanitizing.

Leave `derive_post_metadata` ungated — it uses the parsers but not `ammonia`.

- [ ] **Step 4: Run the full common suite.**

Run: `devtool run -- cargo nextest run -p common`

Expected: PASS. Pre-existing `render` tests that assert exact HTML may now see
sanitized output; if any fail, confirm the change is sanitization (not
corruption) before adjusting the expectation.

- [ ] **Step 5: Confirm call sites still build.**

`render()`'s three call sites (`post_service.rs:79`, `:169`, `:331`) and two in
`test_support.rs` (`:1017`, `:1373`) need no change — the signature is unchanged
and `storage` enables the feature.

Run: `devtool run -- cargo nextest run -p storage`

Expected: PASS.

- [ ] **Step 6: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`fix(common): sanitize rendered post HTML in all three formats (#445)`.

---

### Task 4: Add `sqlx::Decode` and decode into the newtype

**Files:** `common/src/render.rs`, `storage/src/helpers.rs`,
`storage/src/posts.rs`

**Interfaces:** Produces `impl sqlx::Decode for RenderedHtml` under the existing
`sqlx` feature, constructing the private field directly — it uses neither
`sanitize` nor `from_trusted`.

- [ ] **Step 1: Rewrite the rationale comment.**

`common/src/render.rs:185-191` currently explains why there is **no** `Decode`.
Replace it with why there now **is** one, and what that costs: a decode blesses
any text column typed as `RenderedHtml`, accepted because writes sanitize and
the gate enforces it (spec D3). Record that a _sanitizing_ decode was considered
and rejected — no production data to heal, and it would cost an html5ever parse
per post per read forever.

- [ ] **Step 2: Implement `Decode` inside the existing
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

- [ ] **Step 3: Decode the column directly.**

`storage/src/helpers.rs`: change `PostRow.rendered_html` from `String` to
`RenderedHtml` (joining every other domain column). In `build_post_record`,
replace `rendered_html: RenderedHtml::from_trusted(row.rendered_html)` with
`rendered_html: row.rendered_html`, and update the comment above it — the
"trusted rebuild" note no longer describes what happens.

- [ ] **Step 4: Correct the aspirational storage comments.**

`storage/src/posts.rs`: the doc comments calling the column "Sanitized HTML
rendering" are now true. Reword so they state the guarantee and where it comes
from (`RenderedHtml::sanitize`) rather than reading as an unbacked label.

- [ ] **Step 5: Run the storage suite.**

Run: `devtool run -- cargo nextest run -p storage`

Expected: PASS. A decode-type mismatch surfaces here as a column-decode error.

- [ ] **Step 6: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`refactor(storage): decode rendered_html directly into RenderedHtml (#445)`.

---

### Task 5: Tighten the gate to one door

**Files:** `xtask/src/steps/rendered_html_from_trusted_check.rs`,
`common/src/render.rs`

**Interfaces:** `ALLOWED_FNS` reduces to `["deserialize_rendered_html"]`; the
recovery message names `RenderedHtml::sanitize`.

- [ ] **Step 1: Write the negative test first.**

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

- [ ] **Step 2: Run it, verify it passes already.**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml an_inbound_shaped_fn`

Expected: PASS — the gate already flags non-allowlisted callers. The test pins
that behavior against future regression rather than driving new code.

- [ ] **Step 3: Shrink the allowlist.**

Remove `"build_post_record"` — after Task 4 it no longer calls `from_trusted`.
Keep `"deserialize_rendered_html"`.

Fix the stale location comment while here: it says `web/src/posts/mod.rs`, but
`deserialize_rendered_html` lives in `common/src/render.rs` and is used by
`common/src/seed.rs`.

- [ ] **Step 4: Point the recovery message at the new door.**

The message currently ends "otherwise obtain the `RenderedHtml` from `render()`
instead." Reword: data from **outside** must go through
`RenderedHtml::sanitize`; `from_trusted` is only for our own prior output
round-tripping.

Update the assertion in `problems_reports_file_line_and_recovery` if it pins
wording.

- [ ] **Step 5: Rewrite `from_trusted`'s doc comment.**

`common/src/render.rs:99-102` predates any sanitizer. State that it _inherits_
safety from our own prior `sanitize` output round-tripping through our store or
wire, and contrast it with `sanitize`, which _establishes_ safety. Note it is
the sole remaining door and that the gate enforces this.

- [ ] **Step 6: Run the gate against the real tree.**

Run: `devtool run -- cargo xtask check`

Expected: PASS, including `rendered-html-from-trusted`. A failure here means a
`from_trusted` call survives somewhere Task 4 was meant to remove it.

- [ ] **Step 7: Commit.**

`refactor(xtask): reduce the from_trusted allowlist to one door (#445)`.

---

### Task 6: End-to-end regression test

**Files:** `storage/src/post_service.rs` (in-file `#[cfg(test)]`) or the storage
integration suite, following the crate's existing convention.

**Interfaces:** Consumes `storage::perform_post_creation`; asserts the persisted
`rendered_html` carries no active markup. This is AC2.

- [ ] **Step 1: Write the test.**

Create a post whose body contains `<script>alert(1)</script>` and an
`<img src=x onerror=alert(1)>` via the real creation path, read the record back,
and assert the stored `rendered_html` contains neither `<script` nor `onerror`.

**Follow the dual-backend template** (`CONTRIBUTING.md` "backend parity") — a
bare `#[tokio::test]` here fails the `test-backend-pattern` guard.

Prefer an existing fixture from `storage::test_support` over hand-rolling setup.

- [ ] **Step 2: Run it.**

Run: `devtool run -- cargo nextest run -p storage rendered_html`

Expected: PASS on both backends. If it fails, the creation path is not reaching
the sanitizing `render()`.

- [ ] **Step 3: Gate and commit.**

`devtool run -- cargo xtask check`, then commit:
`test(storage): assert malicious post bodies are sanitized end-to-end (#445)`.

---

### Task 7: Draft the ADR

**Files:** `docs/adr/drafts/rendered-html-sanitization.md` (numberless —
`cargo xtask adr promote` numbers it at ship)

- [ ] **Step 1: Write the draft.**

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

- [ ] **Step 2: Format and commit.**

Run `prettier -w` on the draft (the pre-commit hook will otherwise restage it).
`devtool run -- cargo xtask check`, then commit:
`docs(adr): draft the rendered-HTML sanitization decision (#445)`.

---

### Task 8: Final verification

- [ ] **Step 1: Re-verify the wasm graph.**

Run: `devtool run -- cargo tree -p csr --target wasm32-unknown-unknown`; grep
for `ammonia`. Expected: no match (AC7).

- [ ] **Step 2: Confirm one production `from_trusted` door remains.**

Grep `from_trusted` across `common/src`, `storage/src`, `server/src`, `web/src`,
excluding `#[cfg(test)]` sites. Expected: only `deserialize_rendered_html` (AC5,
AC4).

- [ ] **Step 3: Full local gate.**

Run: `devtool run -- cargo xtask validate`

Expected: PASS. Per the milestone-11 experience, the four concurrent e2e VMs can
starve under host load — on an exit-124 timeout, fall back to a single
`cargo xtask e2e sqlite chromium` and let CI's matrix be authoritative
(ADR-0034).

- [ ] **Step 4: Conformance check.**

Walk AC1–AC10 in the spec and confirm each. Note any not literally met and why.

- [ ] **Step 5: Handoff.**

Invoke `jaunder-ship`: whole-branch review, archive spec + plan, push, PR,
monitor CI, halt before merge. This is security-adjacent, so it gets a full
review regardless of diff size.
