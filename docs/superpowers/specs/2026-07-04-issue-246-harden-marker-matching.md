# Spec — #246: harden `cov:ignore` line matcher + `crap:allow` span precision

- **Issue:** jaunder-org/jaunder#246 (milestone "Verify-gate hardening")
- **From:** #231 / PR #244 review (nits #5, #6). Two low-risk robustness nits in
  the stateless coverage gate's marker matching. Neither is a soundness hole
  today (defaults fail-closed; 0 CRAP fails), both are footguns.
- **Status:** approved

## Context

Two marker matchers in the stateless coverage gate use loose substring/heuristic
matching. See the issue for the problem statements; this spec is the "how".

- **`cov:ignore` (line form)** — `xtask/src/coverage/report.rs`. After
  [`line_comment`] extracts a report line's _real_ trailing `//` comment
  (already string-/char-/doc-comment-aware), the marker checks are bare
  `comment.contains("cov:ignore")` (and `-start`/`-stop`). So an executable line
  whose genuine comment merely _mentions_ the token —
  `do_work() // unlike the cov:ignore path` — is silently dropped from
  measurement (a coverage hole).
- **`crap:allow` span** — `xtask/src/coverage/crap.rs`. The override window is
  `CRAP_SPAN_ABOVE = 12` lines above cargo-crap's reported line down through
  `function_body_end`, which brace-matches by counting `{`/`}` in raw text
  (including braces inside strings/comments). So a `crap:allow` on a _preceding_
  function within 12 lines can waive a _following_ over-threshold function, and
  string/comment braces can mis-bound the span.

## Load-bearing subtlety (crap side)

cargo-crap's reported `line` is only _approximately_ the signature (documented
at `crap.rs:38-44`): for `server/src/main.rs::run` it lands _inside the doc
comment_, six lines above the signature; for `test-support/src/main.rs::main` it
is the `// cov:ignore-start` comment (line 80) sitting _between_ the
`#[tokio::main]` attribute (line 79) and `async fn main` (line 81). Crucially,
**a function's `syn` span includes its outer attributes and doc comments**
(`///` → `#[doc]`, plus `#[tokio::main]` etc.), so both reported lines fall
_within_ the function's span — the `contains` match (below) resolves both real
cases directly. The fix must not assume the reported line is the `fn` line, but
it also need not guess: the fuzzy line lands inside the parsed span.

## Decisions (resolved in design interview)

1. **`cov:ignore`: anchored first-token match.** Replace the three `contains`
   checks with: the marker must be the **first whitespace-delimited token** of
   the extracted comment — `comment.split_whitespace().next() == Some(marker)`.
   This drops the line for `// cov:ignore`, `//cov:ignore`, and
   `// cov:ignore <trailing note>`, but NOT for an incidental mention
   (`// unlike the cov:ignore path`). Applied identically to all three markers
   (`cov:ignore`, `cov:ignore-start`, `cov:ignore-stop`); the existing
   `-start`/`-stop`-before-line-form ordering and the `line_comment` extraction
   (strings, char literals, `///`/`//!` doc comments) are unchanged.

2. **`crap:allow`: parse-based exact span via `syn`.** `syn` (2, `full`) and
   `proc-macro2` (`span-locations`) are already xtask deps, and `exempt.rs`
   already parses coverage sources with `syn::parse_file` +
   `syn::visit::Visit` + `proc_macro2::Span::start()/end().line`. Mirror that:
   - Visit `ItemFn`, impl methods (`ImplItemFn`), and trait default methods
     (`TraitItemFn` with a body). For each, record `start_line` (min over the
     fn's attribute + signature spans — outer doc comments are `#[doc]`
     attributes, so the span already covers a doc-comment / attribute header)
     and `end_line` (the block's closing brace).
   - **Map cargo-crap's reported `line` to a function by line containment** (the
     line is fuzzy but always lands within the parsed span; naming format across
     tools is not relied on): among the function spans whose
     `[start_line, end_line]` **contains** the reported line, pick the
     **innermost** (smallest span) — so a nested `fn inner` resolves to `inner`,
     never its enclosing `outer` (this is what keeps AC3's no-bleed guarantee
     closed under nesting). If **no** span contains the reported line (does not
     occur for today's sources — both live cases are contained), treat it as
     **no override** (fail-closed, per Decision 3) rather than widening the
     search.
   - Search **only the lines that belong to that function** for a valid
     `crap:allow` marker (unchanged `is_allow_marker`: `// crap:allow:` +
     non-empty reason) — i.e. lines within its span **whose own innermost span
     is this function**, so the interior of any nested `fn` is excluded (a
     contiguous `[start, end]` slice would subsume a nested fn's body and
     re-open the bleed). This eliminates the naive brace scan entirely (the
     closing brace comes from the parsed AST, so braces in strings/comments can
     never mis-bound it) and the cross-function bleed (the scanned lines are
     exactly one function's own).
   - Delete `function_body_end`, `CRAP_SPAN_ABOVE`, `CRAP_SPAN_FALLBACK`
     (superseded); the reported `function` name is no longer used for span
     resolution.

3. **Fail-closed + logged warning for the two "can't happen" paths.** If
   `syn::parse_file` fails (essentially impossible — coverage sources are the
   project's own compiling Rust, which `syn` `full` parses), **or** the reported
   line is contained by no function span (also not observed — both live cases
   are contained), treat the function as having **no override** (it fails the
   gate — conservative) and emit a warning naming the file/line (diagnostic
   visibility). No silent pass, and no widening the search to guess a nearby
   function.

4. **No ADR; no separable concerns.** This hardens existing markers within the
   ADR-0050 stateless gate; no architectural decision, nothing to file.

## Design

**`report.rs`** — a small predicate, e.g.
`fn comment_marker_is(comment: &str, marker: &str) -> bool { comment.split_whitespace().next() == Some(marker) }`,
replacing the `c.contains("cov:ignore-start")` (54),
`c.contains("cov:ignore-stop")` (64), and `c.contains("cov:ignore")` (86)
checks. No change to `line_comment`, block/line ordering, or the report-parsing
loop.

**`crap.rs`** — replace `allow_overrides` (and delete `function_body_end` + the
two span constants) with a `syn`-based resolver: parse the file once, collect
`FnSpan { start_line, end_line }` for every function (visitor like
`exempt.rs`'s), resolve the reported line to the innermost containing span (rule
above), and scan `src.lines()` in `[start_line ..= end_line]` (1-based →
0-based) for `is_allow_marker`. Keep the injectable `SourceResolver` and
`is_allow_marker` as-is. Note: `is_allow_marker` stays a substring test
(`.find("// crap:allow:")` + non-empty reason) — deliberately _not_ anchored
like `cov:ignore`, since a `crap:allow` is now confined to a single function's
exact span, so an incidental mention elsewhere can no longer reach it. (An
incidental `// crap:allow:` _inside_ the target function would still count — an
accepted, far narrower footgun than the cross-function bleed this closes; out of
scope to anchor.)

Files touched: `xtask/src/coverage/report.rs`, `xtask/src/coverage/crap.rs`. No
`flake.nix`/CI/threshold change.

## Acceptance criteria (observable)

1. **AC1 — line-form `cov:ignore` is anchored.** In `report.rs`'s parser: a
   report line whose real trailing comment's first token is `cov:ignore` (incl.
   `// cov:ignore reason`) is dropped from the executable set; a line whose
   comment merely _mentions_ the token
   (`do_work() // unlike the cov:ignore path`) is **kept**. Regression tests for
   both, plus the existing string-embedded / `///` / `//!` cases still pass.
2. **AC2 — block markers are anchored.** `cov:ignore-start`/`-stop` open/close a
   block only when the marker is the comment's first token; an incidental
   mention does not open/close a block. Regression test.
3. **AC3 — no cross-function bleed.** (a) A `crap:allow` on a _preceding_
   function (within what was the 12-line window) does **not** waive a
   _following_ over-threshold function — two-function fixture. (b) A
   `crap:allow` inside a _nested_ `fn inner` does **not** waive its enclosing
   `fn outer`, and one in `outer` (outside `inner`) does not waive `inner` — the
   innermost-containing-span rule. Regression tests for both.
4. **AC4 — string/comment braces don't mis-bound.** A function whose body
   contains a `}` inside a string (`let s = "}";`) or a comment (`// }`) is
   bounded by its real closing brace: a `crap:allow` placed _after_ the real
   body does **not** waive it, and one _inside_ does. Regression test.
   (Satisfied by construction — spans come from the AST.)
5. **AC5 — the real reported-line patterns resolve (both via `contains`).**
   Fixtures: (i) reported line _inside a doc-comment_ header above the signature
   (`server::run` style) → contained via `#[doc]` attrs; (ii) reported line on a
   comment _between an outer attribute and the `fn`_ (`test-support main` style:
   `#[tokio::main]` / `// comment` / `fn`) → contained because the attribute
   pulls `start_line` up. Each maps to the intended function and a `crap:allow`
   in its span waives it. Plus (iii) a fail-closed case: a reported line
   contained by **no** function span yields _no override_ (the function fails) —
   documents Decision 3. Regression tests; each names its expected path
   (contains vs fail-closed).
6. **AC6 — no live regression.** The real `test-support/src/main.rs::main`
   `crap:allow` still waives it; `cargo xtask check` coverage stays clean (0
   failures, 0 guard violations, 0 CRAP over threshold).
7. **AC7 — tests are pure/host-side.** New unit tests take source strings (and,
   for crap, injected reports via the existing `AllowSet`/`SourceResolver`
   seam), run under the xtask host suite, deterministic — matching the existing
   `report.rs`/`crap.rs` test style.

## Out of scope

- Any change to marker _syntax_, the CRAP threshold (T=30), or the reason
  requirement.
- Block-comment (`/* … */`) marker support — markers are `//` line comments
  only.
- Anything beyond the two matchers.

## Testing / verification ladder

- New unit tests (AC1–AC5, AC7) via
  `cargo nextest run --manifest-path xtask/Cargo.toml coverage::report` /
  `coverage::crap`.
- `cargo xtask check` green before ship (AC6 — coverage clean; probe/gate
  unaffected).
