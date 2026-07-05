# Harden marker matching (#246) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating a task to a subagent via **jaunder-dispatch**
> when useful). Steps use checkbox (`- [ ]`) syntax.

**Spec:**
[`docs/superpowers/specs/2026-07-04-issue-246-harden-marker-matching.md`](../specs/2026-07-04-issue-246-harden-marker-matching.md)
— the "what/why". This plan is the "how".

**Goal:** Anchor the `cov:ignore` line matcher to a first-token test, and
replace the `crap:allow` fuzzy-window/naive-brace span with syn-parsed exact
function spans (innermost-containing).

**Architecture:** Two independent, self-contained matcher hardenings in
`xtask/src/coverage/`. `report.rs` gains a one-line first-token predicate;
`crap.rs` swaps its hand-rolled span logic for a `syn` visitor mirroring
`exempt.rs`. Both are pure, in-file-unit-tested.

**Tech Stack:** Rust (xtask), `syn` 2 (full/visit) + `proc-macro2`
(span-locations) — already deps.

## Global Constraints

- **`cov:ignore` markers (verbatim):** `cov:ignore`, `cov:ignore-start`,
  `cov:ignore-stop`. Anchored = the marker is the **first whitespace-delimited
  token** of the extracted comment.
- **`crap:allow` marker (unchanged):** `// crap:allow:` + non-empty reason
  (`is_allow_marker` stays a substring test, now confined to one function's
  span).
- **crap span resolution:** innermost `syn` function span (attrs+sig+block)
  **containing** cargo-crap's reported line; delete `function_body_end`,
  `CRAP_SPAN_ABOVE`, `CRAP_SPAN_FALLBACK`; reported `function` name unused.
- **Fail-closed + warn:** on `syn` parse failure OR no containing span → no
  override + `eprintln!` warning naming file/line. No silent pass.
- **No scope creep:** no change to marker syntax, CRAP threshold (30),
  `flake.nix`, or CI.
- **Commits:** run `cargo xtask check` clean first (**jaunder-commit**). **No
  `Co-Authored-By` trailer.**

---

## Review header

**Scope — in:** `report.rs` cov:ignore matcher + `crap.rs` crap:allow span.
**Out:** marker syntax, threshold, `is_allow_marker` anchoring, block-comment
markers, `flake.nix`/CI. **Separable concerns:** none.

**Tasks:**

1. `report.rs` — anchored first-token `cov:ignore` matcher (AC1, AC2).
2. `crap.rs` — syn-parsed innermost function span for `crap:allow` (AC3, AC4,
   AC5), delete the old heuristic.

**Key risks/decisions:** the crap matching is line-only (robust to cargo-crap's
naming) and innermost-wins (closes nested-fn bleed); both live cases resolve via
`contains` (verified in review). AC6 (no live regression — `test-support main`
still waived, coverage stays clean) is checked by `cargo xtask check` at each
task's commit.

---

### Task 1: Anchor the `cov:ignore` line matcher (`report.rs`)

**Files:**

- Modify: `xtask/src/coverage/report.rs` (the three marker checks at `:54`,
  `:64`, `:86`; add a predicate)
- Test: in-file `#[cfg(test)] mod tests` (already present,
  `parse_text_report`-based fixtures)

**Interfaces:**

- Consumes:
  `parse_text_report(report: &str, repo_root: &str) -> anyhow::Result<Vec<FileCoverage>>`,
  `line_comment` (unchanged).
- Produces: `fn comment_marker_is(comment: &str, marker: &str) -> bool`
  (private).

- [x] **Step 1: Write the failing tests** (report-fixture style, mirroring
      `line_marker_ignored_only_as_real_comment`)

```rust
#[test]
fn incidental_mention_in_real_comment_is_kept() {
    // An executable line whose GENUINE trailing comment merely mentions the token
    // must NOT be dropped (the #246 footgun).
    let report = "\
/repo/a.rs:
    1|     0|    do_work() // unlike the cov:ignore path
    2|     0|    boom() // cov:ignore
";
    let files = parse_text_report(report, "/repo").unwrap();
    let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
    assert_eq!(lines, vec![1]); // line 2 dropped (anchored marker), line 1 kept
}

#[test]
fn line_marker_with_trailing_note_is_dropped() {
    let report = "\
/repo/a.rs:
    1|     0|    boom() // cov:ignore reason here
";
    let files = parse_text_report(report, "/repo").unwrap();
    assert!(files[0].lines.is_empty()); // first token is the marker → dropped
}

#[test]
fn block_markers_are_anchored_not_incidental() {
    // A comment mentioning cov:ignore-start as non-first-token must NOT open a block.
    let report = "\
/repo/a.rs:
    1|     0|    keep() // see the cov:ignore-start docs
    2|     0|    also_keep()
";
    let files = parse_text_report(report, "/repo").unwrap();
    let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
    assert_eq!(lines, vec![1, 2]); // no block opened; both lines measured
}

#[test]
fn comment_marker_is_matches_first_token_only() {
    assert!(comment_marker_is(" cov:ignore", "cov:ignore"));
    assert!(comment_marker_is(" cov:ignore trailing", "cov:ignore"));
    assert!(comment_marker_is("cov:ignore", "cov:ignore")); // no leading space
    assert!(!comment_marker_is(" unlike the cov:ignore path", "cov:ignore"));
    assert!(comment_marker_is(" cov:ignore-start", "cov:ignore-start"));
    assert!(!comment_marker_is(" cov:ignore-start", "cov:ignore")); // distinct token
}
```

- [x] **Step 2: Run, verify FAIL**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::report`
Expected: FAIL — `comment_marker_is` undefined; the incidental-mention test
fails under today's `contains`.

- [x] **Step 3: Implement**

Add the predicate and swap the three checks. Body is pinned by
`comment_marker_is_matches_first_token_only`:

```rust
/// True iff `marker` is the first whitespace-delimited token of `comment` (the text
/// after `//`, from `line_comment`). Anchors marker recognition so an incidental
/// mention in prose (`// unlike the cov:ignore path`) is inert (#246).
fn comment_marker_is(comment: &str, marker: &str) -> bool {
    comment.split_whitespace().next() == Some(marker)
}
```

Then: `report.rs:54` `c.contains("cov:ignore-start")` →
`comment_marker_is(c, "cov:ignore-start")`; `:64`
`c.contains("cov:ignore-stop")` → `comment_marker_is(c, "cov:ignore-stop")`;
`:86` `c.contains("cov:ignore")` → `comment_marker_is(c, "cov:ignore")`.
(Ordering unchanged: `-start`/`-stop` still checked before the line form.)

- [x] **Step 4: Run, verify PASS**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::report`
Expected: PASS (new tests + all existing `report` tests, incl.
`marker_in_string_literal_does_not_suppress`).

- [x] **Step 5: Commit**

```bash
git add xtask/src/coverage/report.rs
git commit -m "fix(coverage): anchor cov:ignore matcher to the comment's first token (#246)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 2: syn-parsed exact `crap:allow` span (`crap.rs`)

**Files:**

- Modify: `xtask/src/coverage/crap.rs` (rewrite `allow_overrides`; delete
  `function_body_end`, `CRAP_SPAN_ABOVE`, `CRAP_SPAN_FALLBACK`; add a visitor +
  `resolve_span`; update the `evaluate_crap` call site)
- Test: in-file `#[cfg(test)] mod tests` (present; `ent()` / `AllowSet::new`
  helpers)

**Interfaces:**

- Consumes: `Entry { file, function, line: i64, crap }`,
  `evaluate_crap(entries, &AllowSet)`, `is_allow_marker` (unchanged), and the
  exempt.rs pattern (`syn::parse_file`, `syn::visit::Visit`,
  `syn::spanned::Spanned`, `proc_macro2::Span::start()/end().line`, recursing
  via `syn::visit::visit_*`).
- Produces:
  - `fn fn_spans(src: &str) -> syn::Result<Vec<(usize, usize)>>` — every
    function's 1-based `(start_line, end_line)`, attrs through closing brace.
  - `fn resolve_span(spans: &[(usize, usize)], line: usize) -> Option<(usize, usize)>`
    — innermost containing span.
  - `fn allow_overrides(src: &str, line: i64, file: &str) -> bool` (rewritten;
    gains `file` for the warning).

- [ ] **Step 1: Write the failing tests**

```rust
// helper: run evaluate_crap over one over-threshold entry with injected source,
// returning whether it was WAIVED (not in the fail list).
fn waived(src: &'static str, line: i64) -> bool {
    let entries = vec![ent("a.rs", "f", line, 99.0)];
    let allow = AllowSet::new(move |_| Some(src.to_string()));
    evaluate_crap(&entries, &allow).is_empty()
}

#[test]
fn marker_inside_target_function_waives() {
    let src = "\
fn f() {
    // crap:allow: reason
    body();
}
";
    assert!(waived(src, 1)); // reported at the fn line
}

#[test]
fn no_bleed_from_preceding_function() {
    // crap:allow belongs to g (above); it must NOT waive f. Within the old 12-line window.
    let src = "\
fn g() {
    // crap:allow: for g only
}
fn f() {
    body();
}
";
    assert!(!waived(src, 4)); // f starts at line 4; g's marker must not reach it
}

#[test]
fn no_bleed_across_nested_functions() {
    // marker in inner must not waive outer, and vice versa (innermost-containing rule).
    let src = "\
fn outer() {
    fn inner() {
        // crap:allow: inner only
        x();
    }
    y();
}
";
    assert!(!waived(src, 1));  // outer (line 1) not waived by inner's marker
    assert!(waived(src, 2));   // inner (line 2) IS waived by its own marker

    // Reverse direction (AC3b): a marker in OUTER's body must not waive INNER.
    let src2 = "\
fn outer() {
    fn inner() {
        x();
    }
    // crap:allow: outer only
    y();
}
";
    assert!(!waived(src2, 2)); // inner (line 2) not waived by outer's marker
    assert!(waived(src2, 1));  // outer (line 1) waived by its own marker
}

#[test]
fn brace_in_string_or_comment_does_not_misbound() {
    // A `}` in a string/comment must not end the span early; a marker AFTER the real
    // body must not waive; syn spans make this automatic.
    let src = "\
fn f() {
    let s = \"}\";  // } not a real close
    body();
}
// crap:allow: this is OUTSIDE f
fn g() { z(); }
";
    assert!(!waived(src, 1)); // f not waived by the marker below its real close
}

#[test]
fn reported_line_inside_doc_header_resolves() {
    // AC5(i): cargo-crap points into the doc-comment header (contained via #[doc]).
    let src = "\
/// doc line one
/// doc line two
/// crap:allow lives below, not here
fn run() {
    // crap:allow: reason
    a();
}
";
    assert!(waived(src, 2)); // reported inside the doc header → contained → waived
}

#[test]
fn reported_line_between_attr_and_fn_resolves() {
    // AC5(ii): test-support main shape — attribute pulls start_line up over the comment.
    let src = "\
#[tokio::main]
// cov:ignore-start
async fn main() {
    // crap:allow: harness entrypoint
    a();
}
";
    assert!(waived(src, 2)); // reported at the comment (line 2), contained via the attr
}

#[test]
fn no_containing_span_is_fail_closed() {
    // AC5(iii): a reported line outside every function span → no override (fails).
    let src = "\
const X: u32 = 1;
fn f() { a(); }
";
    assert!(!waived(src, 1)); // line 1 is in no fn span → fail-closed
}

#[test]
fn resolve_span_picks_innermost() {
    // outer [1..7], inner [2..5]; line 3 → inner.
    let spans = vec![(1usize, 7usize), (2, 5)];
    assert_eq!(resolve_span(&spans, 3), Some((2, 5)));
    assert_eq!(resolve_span(&spans, 6), Some((1, 7)));
    assert_eq!(resolve_span(&spans, 9), None);
}
```

- [ ] **Step 2: Run, verify FAIL**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::crap`
Expected: FAIL — `fn_spans`/`resolve_span` undefined; the
nested/brace/fail-closed cases fail under today's window+brace logic.

- [ ] **Step 3: Implement**

Add the visitor + resolver and rewrite `allow_overrides`; delete
`function_body_end` and the two `CRAP_SPAN_*` constants. Signatures pinned by
the tests; the visitor mirrors `exempt.rs`:

```rust
use proc_macro2::Span;
use syn::spanned::Spanned;

/// 1-based (start_line, end_line) — attributes/doc through the closing brace — for
/// every function in `src` (free fns, impl methods, trait default methods),
/// including nested fns (the visitor recurses). Mirrors `exempt.rs`'s syn visitor.
fn fn_spans(src: &str) -> syn::Result<Vec<(usize, usize)>> {
    let file = syn::parse_file(src)?;
    let mut out = Vec::new();
    let mut v = FnSpanVisitor { out: &mut out };
    syn::visit::visit_file(&mut v, &file);
    Ok(out)
}

struct FnSpanVisitor<'a> {
    out: &'a mut Vec<(usize, usize)>,
}

/// (start_line, end_line): the min of the fn's attribute + signature span starts
/// (so an outer doc/attr header is included) through the block's closing brace.
fn bounds(attrs: &[syn::Attribute], sig: Span, block: Span) -> (usize, usize) {
    let start = attrs
        .iter()
        .map(|a| a.span().start().line)
        .chain(std::iter::once(sig.start().line))
        .min()
        .unwrap_or_else(|| sig.start().line);
    (start, block.end().line)
}

impl<'ast> syn::visit::Visit<'ast> for FnSpanVisitor<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.out.push(bounds(&f.attrs, f.sig.span(), f.block.span()));
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.out.push(bounds(&f.attrs, f.sig.span(), f.block.span()));
        syn::visit::visit_impl_item_fn(self, f);
    }
    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if let Some(block) = &f.default {
            self.out.push(bounds(&f.attrs, f.sig.span(), block.span()));
        }
        syn::visit::visit_trait_item_fn(self, f);
    }
}

/// The innermost (smallest) span containing 1-based `line`, or `None`.
fn resolve_span(spans: &[(usize, usize)], line: usize) -> Option<(usize, usize)> {
    spans
        .iter()
        .filter(|&&(s, e)| s <= line && line <= e)
        .min_by_key(|&&(s, e)| e - s)
        .copied()
}

/// Does `src` carry a valid `crap:allow` override within the span of the function
/// that contains cargo-crap's reported `line`? Fail-closed (no override) on a parse
/// failure or a line contained by no function span — with a warning, since neither
/// happens for the project's own compiling sources.
fn allow_overrides(src: &str, line: i64, file: &str) -> bool {
    let line = line.max(1) as usize;
    let spans = match fn_spans(src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("xtask: coverage: crap:allow span skipped — {file} did not parse: {e}");
            return false;
        }
    };
    let Some((start, end)) = resolve_span(&spans, line) else {
        eprintln!("xtask: coverage: crap:allow — no function span contains {file}:{line}");
        return false;
    };
    let lines: Vec<&str> = src.lines().collect();
    // Scan only lines that belong to THIS function — i.e. whose own innermost span
    // is the resolved one. This excludes the interior of any nested fn (whose lines
    // resolve to the nested, smaller span), so a nested fn's marker cannot waive its
    // enclosing fn (AC3b). A contiguous `start..=end` slice would subsume nested fns.
    (start..=end.min(lines.len()))
        .filter(|&ln| resolve_span(&spans, ln) == Some((start, end)))
        .any(|ln| is_allow_marker(lines[ln - 1]))
}
```

Update the `evaluate_crap` call site (`crap.rs:101-103`):
`allow_overrides(&src, e.line)` → `allow_overrides(&src, e.line, &e.file)`.

- [ ] **Step 4: Run, verify PASS**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::crap`
Expected: PASS (new tests + existing crap tests, incl. the empty-reason and
existing-marker cases).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/crap.rs
git commit -m "fix(coverage): bound crap:allow to the exact syn function span (#246)"
```

Run `cargo xtask check` first (**jaunder-commit**) — this also verifies **AC6**:
the real `test-support/src/main.rs::main` waiver still holds and coverage stays
clean (0 failures, 0 CRAP over threshold).

---

## Self-review

- **Spec coverage:** AC1 → T1 (incidental-kept, trailing-note-dropped); AC2 → T1
  (block-anchored); AC3 → T2 (preceding + nested); AC4 → T2
  (brace-in-string/comment); AC5 → T2 (doc-header, attr-then-comment,
  fail-closed); AC6 → T2/S5 (`cargo xtask check`); AC7 → both (in-file pure unit
  tests). All mapped.
- **Placeholders:** none — real Rust + exact `cargo nextest` commands.
- **Type consistency:** `comment_marker_is(&str,&str)->bool` (T1);
  `fn_spans(&str)->syn::Result<Vec<(usize,usize)>>`,
  `resolve_span(&[(usize,usize)],usize)->Option<(usize,usize)>`,
  `allow_overrides(&str,i64,&str)->bool` (T2) match their call sites; `bounds`
  takes `proc_macro2::Span`s consistent with `sig.span()`/`block.span()`.
