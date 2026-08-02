# Spec — #778: gate exemptions become in-source site markers

- Issue: [#778](https://github.com/jaunder-org/jaunder/issues/778)
- Milestone: Web: canonical Leptos CSR convergence
- Date: 2026-08-01

## Problem

The three ident-keyed XSS gates — `raw-html-door`, `html-sink`,
`rendered-html-from-trusted` — exempt production sites through a central
allowlist **keyed by enclosing function name**. `rendered-html-from-trusted`
carries the weakest form (`ALLOWED_FNS`, no multiplicity), so a second
`from_trusted` inside `deserialize_rendered_html` passes silently — an ADR-0085
principle-4 region exemption.

The issue originally proposed adding a multiplicity, matching the two siblings'
`ident_gate::Allowed`. That is the **wrong key**, not a smaller version of the
right one. Principle 4's rule is _"scopes each allowlist entry to a single site,
never to a region"_; multiplicity is its **fallback**, for _"where a population
genuinely contains indistinguishable sites."_ `deserialize_rendered_html` holds
exactly one site, perfectly distinguishable. Multiplicity would leave the entry
region-scoped with a cardinality assertion attached, and three failure modes
open:

1. **Swap, don't add** — replacing the door inside an allowed fn keeps the count
   at 1 and passes. The count is blind to _which_ site.
2. **Shadow with deletion** — a same-named fn in a second file while the
   original is deleted keeps the tree-wide total matching, and passes.
3. **Rename / move** — the key is a name, so refactoring silently transfers or
   drops an exemption.

Separately, the declared list can go stale, which is the sole reason
`Gate::problems`' reconciliation pass exists.

This repo already solved the same problem the other way: coverage exempts by an
in-source `cov:ignore` marker on the line (ADR-0050). That inconsistency had
never been argued, only inherited.

**What markers close, precisely.** Failure modes 2 and 3 go away entirely —
there is no name to shadow and no key to break. Failure mode 1 is **narrowed,
not closed**: the exemption shrinks from a whole function to a single line, so
adding a site no longer hides, but swapping the _value_ that flows into an
already-marked site still passes. No gate here has a call graph (`ident_gate`'s
unreadable class 3), so that window cannot be shut by this change and must not
be claimed as closed.

## Decision

**Gate exemptions become per-site marker comments on the line immediately above
the site, read by the gate.** The key is one line — no fn name, no path, no
ordinal — so scoping is structural rather than bookkept, it survives rename and
file move, and the exempt set becomes **derived rather than declared**.

**Honestly stated:** the key is not literally "the site" but "the line the
marker points at". That line holds exactly one site or the gate fails, so the
binding stays 1 marker : 1 site and is never a region — but it is one level of
indirection off the strongest possible claim, and this spec does not pretend
otherwise.

This applies to all three gates together; converting one would leave
`ident_gate` running two exemption mechanisms at once.

### Marker form

`// <step>:allow <reason>` as a comment line of its own, **directly above** the
site, where `<step>` is the gate's own `Gate::step` value:

- `// raw-html-door:allow <reason>`
- `// html-sink:allow <reason>`
- `// rendered-html-from-trusted:allow <reason>`

Recognition anchors on the comment's first whitespace-delimited token, reusing
the coverage gate's `line_comment` / `comment_marker_is` helpers
(`xtask/src/coverage/report.rs`), promoted into a shared module along with their
existing tests.

Deriving the token from `Gate::step` means there is no separate marker const and
no way for the marker name to drift from the gate name. Per-gate tokens (rather
than one shared `xss:allow`) matter because a single line can hold two
populations — e.g.
`view! { <div inner_html=PreEscaped(x).into_string()></div> }` is a sink _and_ a
raw door — and one shared marker would silence both from one reason.

**Why above the site rather than trailing it — measured, not chosen.** The
obvious position is a trailing comment on the site's own line, and that is what
this spec originally required. It does not survive the formatters, which own
that position: `rustfmt` pushes a comment trailing an opening `{` down onto the
first line of the block, and `leptosfmt` lifts or drops one depending on where
it sits in a `view!` body. Written trailing, **7 of the 12 live sites relocate**
— some above, some below, deterministically per syntactic context but not by any
rule an author could predict.

Written as a standalone comment line directly above the site, **all twelve stay
put**, across repeated `cargo xtask check` runs, including the two
`fn from_trusted` signatures (where the marker sits between `#[must_use]` and
`pub fn`) and all five `view!`-related sinks. That is the one position both
formatters preserve, so it is the position the gate reads. Trailing is
deliberately **not** accepted: it is exactly the form that silently moves, and
accepting it would invite writing a marker that stops working on the next
format.

**`line_comment` must be hardened before it polices a security gate.** Its
current doc concedes _"Raw strings are not specially handled — rare in report
lines, and a best-effort scan is sufficient here."_ That is true for coverage
report lines and false here: a raw string on a site line containing the text
`// html-sink:allow …` would be read as a real marker, which is a **fail-open**.
Raw-string handling (`r"…"`, `r#"…"#` and friends) is added as part of the
promotion.

### Rules

- **Line form only. No block form.** `cov:ignore-start`/`-stop` is precisely the
  region exemption being removed.
- **A reason is required.** A bare marker fails. Nothing beyond non-emptiness is
  checked — prose quality is not machine-checkable and a length floor only
  teaches padding.
- **The line a marker points at must hold exactly one site of that gate.** Two
  or more is a failure telling the author to split the line, so "one marker =
  one site" holds by construction rather than by convention.
- **An orphan marker fails.** A marker whose next line holds no site of its gate
  — because the site was deleted, or moved — is a live, pre-approved exemption
  waiting for a future edit to land on it. This is also the _only_ property of a
  written exemption a machine can re-check (see "What re-checking is possible"
  below), so it is where the strictness budget goes.
- **The site is the line the gate reports** — the matched ident's span line, not
  the first or last line of the enclosing statement. The marker goes on the line
  immediately before it. The failure message prints the site as `file:line`, so
  the author is told exactly which line to sit above.
- **No comment-skipping.** The site must be on the marker's very next line. A
  blank line, or a second comment, between marker and site makes the marker an
  orphan and the site unmarked — two loud failures rather than one silent drift.

### Implementation seam

`mentions(source, &population) -> Vec<Mention>` stays as it is: pure,
marker-unaware, and the shared scan. A new classifier in `ident_gate` takes
`(source, &[Mention], marker_token)` and returns both the unexempted sites and
the census (`(line, function, reason)` for each marked site). `Gate::problems`
calls it per file; `Gate::violations(&str)` — the `#[cfg(test)]` single-source
convenience — calls it too, so every acceptance criterion below remains
unit-testable from a bare source string, which is how all three gates' existing
tests are written.

This is stated because it is the one mechanical choice with consequences:
putting the marker check inside `mentions` instead would break that test seam.

### Population change: `EXEMPT_QUALIFIERS` is deleted

`rendered-html-from-trusted` currently skips every `ContentType::from_trusted`
site tree-wide via a **pattern on the qualifier**. That is the shape ADR-0085's
Context names — _"Moving the pattern match from the violation path to the
exemption path hides the blind spot; it does not remove it"_ — and it fails
**open** asymmetrically: the leaf name is deliberately guarded when aliased
(`use … as`), but aliasing the _qualifier_ (`use RenderedHtml as ContentType`)
hands out the exemption. It was a reasonable shortcut when an exemption cost a
central entry; markers make a per-site exemption nearly free, removing the
reason for it.

Deleting it makes the population `AnyOf(&["from_trusted"])`, which collapses all
three gates onto one shape and deletes the custom `TrustedDoor` `Population`,
the `expr_path` qualifier logic, and `macro_qualifier_is_exempt`.

**Consequence A — definition sites enter the population.** `syn` visits a fn's
own `sig.ident`, so `pub fn from_trusted` is now a hit. Two sites are affected
(`common/src/render.rs`, `common/src/media.rs`); they take markers like any
other. The sibling gates are unaffected — `inner_html`, `set_inner_html` and
`PreEscaped` are all declared outside this tree, so `from_trusted` is the only
guarded ident the repo itself declares. This is a deliberate behavior change,
and it fails closed.

**Consequence B — the gate's prose must stop naming one type.** The current
verdict ("`RenderedHtml::from_trusted` … is not an allowlisted trusted-rebuild
door — a raw string minted here is emitted unescaped (XSS)") will now fire at
`ContentType::from_trusted` call sites and at `fn from_trusted` definitions,
where every clause of it is false. The gate polices an **ident**, not one type's
door, and its `Report` must say so.

### Failure output

`Report::noun` and `Report::vanished` exist only for the reconciliation line and
are deleted with it. `Report::recovery` stops dumping a declared allowlist and
instead ends by instructing the author to add `// <step>:allow <reason>` on the
site's line, followed by the **derived** census — every marked site found in
this scan, as `file:line — reason`. The census keeps the one thing the central
list was good at (a reader can ask "is mine like these, and is this set still
small?") while removing the staleness, and it cannot lie.

### What re-checking is possible

The obvious claim — that coverage's markers are safer because the A1 tripwire
re-tests them — is **false**, and this spec does not rest on it. A `cov:ignore`
line is dropped from the executable set at `xtask/src/coverage/report.rs:85-89`
before the gate sees it; the A1 guard only ever fires on _structural_
exemptions. ADR-0050's own consequences say so: _"`cov:ignore` is permanent. A
marked line that later becomes covered and then regresses is never re-flagged."_

The rule that does hold: a machine can keep testing an exemption it
**inferred**, and cannot re-verify one a human **wrote**. No written exemption
in this repo is re-checkable, under either mechanism. What is left is checking
that the exempted thing still exists — which is what the orphan-marker rule
does, and is strictly more than the central list ever offered.

### Accepted costs

- **Lower review weight.** The exemption now lands in the author's own diff
  rather than forcing an edit to a gate file. Mitigated by requiring a reason,
  failing a bare marker, and failing an orphan. Recorded, not solved.
- **A marker is trusted, not verified.** The gate checks that a reason exists
  and that its site still exists; it never checks that the reason is true. The
  central list had this blind spot equally.

## Scope

In scope:

1. Promote `line_comment` / `comment_marker_is` from
   `xtask/src/coverage/report.rs` to a shared module, with their existing tests,
   **plus raw-string handling**. Both consumers hand it a source line string —
   coverage from the llvm-cov report's text column, the gates from the file by
   span line — so the shared surface is
   `marker_on_line(line, token) -> Option<&reason>`; each consumer's policy
   (bare markers, block form, orphan handling) sits above it.
2. `ident_gate`: replace the `Allowed` allowlist and multiplicity reconciliation
   with the marker classifier. Delete `Allowed`, `unjustified`,
   `Mention::top_level`, and the `Population::expr_path` hook with its
   `visit_expr_path` visitor.
3. The three gates: drop `ALLOWLIST` / `ALLOWED_FNS` / `EXEMPT_QUALIFIERS` /
   `TrustedDoor` / `macro_qualifier_is_exempt`; rewrite each `Report`;
   `rendered-html-from-trusted` converts to `Gate` (it currently uses `run_scan`
   directly).
4. Mark every live site in source, carrying the reasons the allowlists held.
5. A new ADR recording the rule and its discriminator.
6. Update the docs that describe the old state (AC22–AC28).

Out of scope:

- Renaming `ContentType::from_trusted` to remove the ident collision entirely.
  Cleaner endpoint; belongs in its own issue since it changes a `common` API for
  a gate's benefit.
- Any change to `sqlx-newtype-decode`, which legitimately uses multiplicity: its
  population _is_ principle 4's indistinguishable-sites fallback case.
- Any change to the coverage gate's own `cov:ignore` vocabulary — bare markers
  stay legal there, and the block form (`cov:ignore-start`/`-stop`, with its
  nesting and unclosed-at-EOF errors) stays coverage-only. The divergence is
  deliberate: stakes set strictness, not mechanism.

  **Coverage does share the marker primitive**, and therefore inherits the
  raw-string hardening. That is a behavior change to the coverage gate, in the
  fail-closed direction: a `//` opening inside `r"…"` / `r#"…"#` stops being
  read as a comment. No live `cov:ignore` marker is expected to depend on the
  old reading, but the implementation confirms that rather than assuming it — a
  coverage marker that stops suppressing is a newly-failing line, which the gate
  will surface loudly.

## Acceptance criteria

Each is stated so conformance can be checked from the tree or from a gate run.

**Mechanism**

- AC1 — A site whose immediately preceding line is `// <step>:allow <reason>`
  passes its gate.
- AC2 — The same site with the marker removed **fails**, and the failure names
  `file:line`.
- AC3 — A bare marker (`// html-sink:allow` with no following text) **fails**.
- AC4 — A marker **trailing the site's own line**, or two or more lines above
  it, does not exempt it; the site fails. (Trailing is the form the formatters
  relocate, so it must never appear to work.)
- AC5 — A marker appearing inside a string literal — **including a raw string**
  (`r"…"`, `r#"…"#`) — or in a doc comment (`///`, `//!`), does not exempt
  anything.
- AC6 — A marker for one gate does not exempt a site belonging to another gate.
- AC7 — A marker whose next line holds two sites of the same gate **fails**,
  with recovery text directing the author to split the line.
- AC8 — A marker whose next line holds no site of its gate **fails**, naming the
  orphan's `file:line`.
- AC9 — For a mention whose enclosing statement spans multiple lines, the marker
  is honored only directly above the **ident's** line — not above the
  statement's first line.
- AC9a — **Formatter stability:** after `cargo xtask check`, every marker is
  still on the line immediately above its site. Re-running the gate moves
  nothing.
- AC10 — Test code (`#[cfg(test)]` module/impl/fn, `#[test]`/`#[rstest]` fn)
  remains exempt without markers, and an orphan marker in test code does not
  fail.
- AC11 — A `syn` parse failure, an unreadable file, and a missing scan root each
  remain hard failures.
- AC12 — AC1–AC9 are exercised by unit tests that call `Gate::violations` with a
  bare source string, matching how the existing gate tests are written.

**Population**

- AC13 — `ContentType::from_trusted` sites are in `rendered-html-from-trusted`'s
  population: each requires its own marker, and removing one fails the gate.
- AC14 — A `from_trusted` **definition** site (`pub fn from_trusted`) is in the
  population and requires its own marker. A test asserts this explicitly,
  replacing `the_definition_site_has_no_path_mention`.
- AC15 — Sites inside `view!` / `html!` macro token streams are still detected,
  and are exempted by a marker on the ident's line.

**Deletion**

- AC16 — No item named `Allowed`, `unjustified`, `top_level`, `expr_path`,
  `visit_expr_path`, `EXEMPT_QUALIFIERS`, `macro_qualifier_is_exempt`,
  `TrustedDoor`, `ALLOWLIST` or `ALLOWED_FNS` is declared in
  `xtask/src/steps/ident_gate.rs`, `rendered_html_from_trusted_check.rs`,
  `html_sink_check.rs` or `raw_html_door_check.rs`. (Other `xtask/src/steps/`
  modules legitimately use several of these names and are out of scope —
  `sqlx_newtype_decode_check.rs`, `sqlx_newtype_bind_check.rs`,
  `server_fn_coverage_check.rs`, and `server_fn_registrar_check.rs`'s own
  `visit_expr_path`.)
- AC17 — All three gates are expressed as a `Gate<AnyOf>`.
- AC18 — `Report` has no `noun` or `vanished` field.

**Content preservation**

- AC19 — Each of the six reasons in the three `ALLOWLIST` / `ALLOWED_FNS` consts
  before the change has, afterwards, a marker at the corresponding site whose
  text carries the same claim (or adjacent prose that does, with the marker
  pointing at it). Checked by diffing the removed consts against the added
  markers site by site.
- AC20 — `html_sink_check`'s module doc states that its sites' reasons all share
  one shape — the injected value is the output of the pure render layer, the
  same fn the projector paints — and that this uniformity is the point. (A
  per-site marker has nowhere to put a statement about the _set_, so this must
  be rehomed rather than dropped.)
- AC21 — Each gate's module doc states its unreadable classes, including: **a
  marker is trusted, not verified**, and **a marked site is exempt regardless of
  what value flows into it** (no call graph).

**Failure output**

- AC22 — A failing run's recovery text instructs the author to add
  `// <step>:allow <reason>` on the site's line.
- AC23 — A failing run's output ends with the derived census of currently-marked
  sites (`file:line — reason`), computed from the scan rather than from a
  declared list.
- AC24 — `rendered-html-from-trusted`'s failure message is accurate at a
  `ContentType::from_trusted` site and at a `fn from_trusted` definition: it
  does not assert that the site mints `RenderedHtml`, nor that a string minted
  there is emitted unescaped.

**Documentation**

- AC25 — A new ADR (draft in `docs/adr/drafts/`, numbered at ship by
  `cargo xtask adr promote`) records: the marker decision and its rules; that a
  machine can re-check an _inferred_ exemption but never a _written_ one, so
  `cov:ignore` is permanent too and re-checkability separates neither
  population; that keying and a derivable census are what decide marker vs.
  central list, with review weight the accepted loss; that stakes set strictness
  rather than mechanism; the deletion of the qualifier-pattern exemption as
  ADR-0085 principle 3; and the two accepted costs.
- AC26 — ADR-0093's "What it creates" paragraph no longer describes
  `ALLOWED_FNS` as outstanding follow-up.
- AC27 — ADR-0085's **Conformance** section records the three ident gates, and
  its Consequences paragraph no longer claims co-location _with the gate_
  discharges the "record why these sites are fine" requirement — the new ADR
  supersedes that sentence. Its six principles are unchanged.
- AC28 — These four stale descriptions of the old mechanism are corrected:
  `ident_gate`'s module doc (the two-layer allowlist model, the `#778`
  references, unreadable class 4); `common/src/media.rs`'s parenthetical
  claiming the `ContentType::` qualifier is exempt; ADR-0080's statement that
  `EXEMPT_QUALIFIERS` is untouched; and ADR-0079 §88-89 plus the mirroring
  comment in `common/src/render.rs` claiming the gate matches `from_trusted`
  **in expression position** (it now matches the ident anywhere, definitions
  included). ADR-0079's residual-risk conclusion is unaffected and stays.
- AC29 — ADR-0050 gains a cross-reference to the new ADR, noting that
  `cov:ignore` is the same written-exemption mechanism priced for a larger,
  lower-stakes population, and that the two now share a marker primitive.
- AC30 — The coverage gate still passes on the unchanged tree after the shared
  helper gains raw-string handling; any `cov:ignore` marker that stops
  suppressing is investigated rather than re-marked.

**Gate**

- AC31 — `cargo xtask validate --no-e2e` is green, with all three gates passing
  against the marked tree.

## Sites to mark

Derived and independently verified 2026-08-01. Line numbers will shift, so the
implementation re-derives the list and treats this as the expected **set**.

| Gate                         | File                           | Site                                                              |
| ---------------------------- | ------------------------------ | ----------------------------------------------------------------- |
| `rendered-html-from-trusted` | `common/src/render.rs`         | `pub fn from_trusted` (definition)                                |
| `rendered-html-from-trusted` | `common/src/render.rs`         | `.map(RenderedHtml::from_trusted)` in `deserialize_rendered_html` |
| `rendered-html-from-trusted` | `common/src/media.rs`          | `pub(crate) fn from_trusted` (definition)                         |
| `rendered-html-from-trusted` | `common/src/media.rs`          | `ContentType::from_trusted(content_type)`                         |
| `rendered-html-from-trusted` | `common/src/media.rs`          | `ContentType::from_trusted("application/octet-stream")`           |
| `rendered-html-from-trusted` | `common/src/feed/feed_path.rs` | `ContentType::from_trusted(literal)`                              |
| `html-sink`                  | `web/src/home/component.rs`    | masthead `inner_html`                                             |
| `html-sink`                  | `web/src/sidebar/component.rs` | anonymous sidebar `inner_html`                                    |
| `html-sink`                  | `web/src/posts/component.rs`   | `PostDisplay` anonymous layout `inner_html`                       |
| `html-sink`                  | `web/src/posts/component.rs`   | `PostDisplay` authored layout `inner_html`                        |
| `html-sink`                  | `web/src/posts/component.rs`   | `permalink_first_paint` `inner_html`                              |
| `raw-html-door`              | `web/src/html.rs`              | `PreEscaped` in `from_rendered_html`                              |

Twelve sites: seven inherited from the three allowlists, five newly required by
the `EXEMPT_QUALIFIERS` deletion. Every other `from_trusted` in the tree is
`#[cfg(test)]` fixture code or doc-comment prose. A count differing from twelve
at implementation time is a finding to investigate, not a number to adjust
silently.

## Notes

**Order matters: markers first.** Markers are inert comments to the current
gates, so adding all twelve while the old machinery is still in place keeps the
tree green. Deleting `EXEMPT_QUALIFIERS` or switching the gates to read markers
_before_ the `common/src/media.rs` and `common/src/feed/feed_path.rs` markers
exist turns the tree red.
