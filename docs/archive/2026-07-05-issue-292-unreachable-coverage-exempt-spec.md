# Spec — issue #292: structurally exempt `unreachable!("msg")` from the coverage gate

- Issue: [#292](https://github.com/jaunder-org/jaunder/issues/292)
- Milestone: Code quality improvement
- Blocks: #245 (its burn-down converts provably-dead lines to
  `unreachable!("msg")` and relies on this exemption to drop their `cov:ignore`
  markers)
- Sibling to: #231 (ADR-0050 stateless gate) / #246 (marker hardening)

## Problem

ADR-0050 gives the stateless coverage gate two exemptions: the `#[component]`
**structural** exemption (`syn`-based, fail-closed,
`xtask/src/coverage/exempt.rs`) and the manual `// cov:ignore` marker. ADR-0050
consequence #3 records a deliberate weakness: `cov:ignore` is **permanent** and
a prose-only promise — a marked line that later becomes covered-then-regresses
is never re-flagged.

Provably-dead lines (e.g. a match arm that domain invariants make unreachable)
are today accepted with `cov:ignore`, inheriting that permanence. There is a
stronger, self-enforcing way to express "this line cannot execute":
`unreachable!("msg")`. If it _is_ reached it panics, the test fails,
`cargo llvm-cov` exits non-zero, and the Nix `coverage` check produces no report
— you cannot silently cheat coverage on live code the way a `cov:ignore` on a
reachable line allows.

## Solution

Add a **third structural exemption**, mirroring `#[component]`: a line whose
source is an `unreachable!(...)` macro invocation **with a non-empty message**
is dropped from the executable set — no marker needed.

Properties (by design, matching `#[component]`):

- **Self-enforcing** — reaching the line panics ⇒ test fails ⇒ no report. Unlike
  a `cov:ignore` on a reachable line, it cannot silently hide live code.
- **Message-required** — mirrors `crap:allow`'s required reason. Bare
  `unreachable!()` stays **measured** (fails the gate), forcing an explicit
  justification.
- **Fail-closed** — a `syn` parse error or any unrecognized form yields **no**
  exemption (lines stay measured → the gate can still FAIL), never a silent
  drop.

### Scope boundaries (deliberate)

- **`unreachable!` only.** `panic!` (frequently a reachable error path) and
  `todo!` / `unimplemented!` (unfinished-work reminders that _should_ fail
  coverage) stay measured.
- **Literal single-segment `unreachable!` only.** `std::unreachable!`, aliases,
  and macro-_generated_ invocations stay measured — a documented fail-closed
  boundary (recognition is `mac.path.is_ident("unreachable")`, not a substring
  scan).
- **Non-empty message.** Emptiness is judged on the macro's token stream: bare
  `unreachable!()` has empty tokens → not exempt; `unreachable!("x")`,
  `unreachable!("{}", n)` have non-empty tokens → exempt.

## Touch points

### 1. `xtask/src/coverage/exempt.rs` — the visitor

Extend `ExemptVisitor` with a `visit_macro` arm. Keep the public
`exempt_lines(src) -> syn::Result<BTreeSet<u32>>` signature unchanged (no
reason-tagging refactor) — `unreachable!` lines join the same returned set as
`#[component]` body lines.

```rust
fn visit_macro(&mut self, mac: &'ast syn::Macro) {
    // Literal `unreachable!` with a NON-EMPTY message → exempt its span.
    // `is_ident` excludes `std::unreachable!` and aliases (fail-closed).
    if mac.path.is_ident("unreachable") && !mac.tokens.is_empty() {
        add_span(self.out, mac.span());        // path + bang + delimiters
        add_span(self.out, mac.tokens.span()); // the (possibly multi-line) message
    }
    syn::visit::visit_macro(self, mac);
}
```

Spanning **both** `mac.span()` and `mac.tokens.span()` guarantees a multi-line
message is fully exempt even if `mac.span()` degrades to the path line on stable
proc-macro2. An `unreachable!` inside a `#[component]` body is already exempt
via the fn-body span — the overlap is harmless. Update the module docs: it is no
longer `#[component]`-only.

### 2. `xtask/src/coverage/gate.rs` — doc wording only

No signature or behavior change. `evaluate` already treats every exempt line
identically (the A1-guard fires on `covered && exempt` regardless of _why_ the
line is exempt). Generalize the module/`evaluate` **doc comments** so they name
both kinds ("a `#[component]` body or an `unreachable!` assertion"). Executable
logic is untouched.

Note (documented, not fixed): the A1-guard on an `unreachable!` line is
defense-in-depth and near-dead in practice — a reached `unreachable!` panics
before any report is produced, so it rarely surfaces as `covered && exempt`. It
is retained (not special-cased out) so both exemption kinds share one code path.

### 3. `xtask/src/coverage/mod.rs` — `failure_report` wording only

Generalize the A1-guard **header** and **guidance** strings to name both
exemption kinds, preserving the substrings existing tests assert (`"A1-guard"`,
`"revisit the exemption"`). No signature change. This is a pure reword inside
the already-tested `guard_violations`-non-empty branch, so existing
`failure_report` tests still cover the new lines. **Only** add a new
`failure_report` test if the reword introduces a genuinely new/untested branch
(it should not — plan will verify via coverage).

### 4. Tests — `xtask/src/coverage/exempt.rs` `#[cfg(test)]`

Add cases (naming the acceptance list):

- `unreachable!("x")` → exempt.
- bare `unreachable!()` → **not** exempt.
- `panic!("x")` → **not** exempt; `todo!()` → **not** exempt.
- multi-line message span → all message lines exempt.
- `std::unreachable!("x")` → **not** matched (stays measured).
- parse-error → fail-closed (`exempt_lines` returns `Err`; existing
  `parse_error_yields_empty` already asserts this — extend only if a distinct
  case adds value).

### 5. Docs

- **`CONTRIBUTING.md`** "Coverage and dependency policy" (§ around line 399):
  add the third exemption path alongside `#[component]` and `cov:ignore` —
  self-enforcing, message-required, fail-closed; note bare `unreachable!()`
  stays measured. Adjust the "two things exempt it" framing (line 412) to three.
- **`docs/adr/0050-stateless-coverage-gate.md`**: amend (direct edit of the
  promoted, accepted ADR — the draft-out-of-git flow is for _new_ ADRs). Record
  the rationale AND the **honest trade-off**: moving lines from text
  `cov:ignore` (immune to parse errors) to a `syn` structural exemption
  **concentrates fail-closed risk** — a single parse error in a file drops _all_
  its `unreachable!` exemptions at once. That is loud and safe (those lines
  revert to measured, so the gate can only FAIL, never silently pass), but it is
  a robustness downgrade versus the pure-text `cov:ignore` path.

## Acceptance criteria (from the issue)

1. `unreachable!("msg")` needs no `cov:ignore` marker; bare `unreachable!()`
   still fails the gate.
2. Scope respected: `panic!` / `todo!` / `unimplemented!` and
   `std::unreachable!` stay measured.
3. Fail-closed behavior preserved (unrecognized/unparseable → measured).
4. CONTRIBUTING coverage section and ADR-0050 updated.
5. The change covers its own new lines (the `exempt.rs` visitor arm is exercised
   by the new tests; the doc rewords sit in already-tested branches). Verified
   green under `cargo xtask check`.

## Out of scope

- #245's actual burn-down (converting real dead lines to `unreachable!("msg")`
  and removing their markers) — this issue only supplies the exemption #245
  depends on.
- Any reason-tagging refactor of `exempt_lines`' return type.
- Exempting `panic!` / `todo!` / `unimplemented!` or non-literal `unreachable!`
  paths.
