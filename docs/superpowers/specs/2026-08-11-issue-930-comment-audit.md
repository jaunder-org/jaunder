# Spec — issue #930: comment audit across the codebase

## Problem

Comments across the repo have drifted from their job. The standard is the
existing one in `CONTRIBUTING.md` ("Comment for intent, not mechanics"): a
comment states **intent** — what the code is expected to do, an informal
specification a reader can judge correctness against — and, where the code takes
a non-obvious path, **why** it is done that way. A comment that only restates
what the code plainly says, narrates history, or buries the reader in an essay
is a defect. Pure why-comments are compliant, not redundant.

## Defect classes

1. **Backward-looking.** Phrased against the past ("no longer does X",
   "previously", "moved from", "used to"). Rewrite the few that carry
   present-tense intent; delete the rest — history lives in `git log`.
2. **Overlong.** Multi-paragraph inline essays that disrupt reading. Shorten
   aggressively; where the detail is load-bearing, move it to an ADR or doc and
   leave a one-line pointer. Carve-outs: module-level `//!` docs are
   comprehensive **by policy** (`CONTRIBUTING.md`: "Every module should have
   comprehensive internal documentation") — audit them for the other defect
   classes, not for length; gate honesty prose mandated by ADR-0085 ("what this
   gate does not claim") is content, not an essay.
3. **ADR-worthy.** Comments that are the _only_ record of a novel decision
   (workaround + root cause, rejected alternative, protocol quirk). Promote to a
   numberless draft in `docs/adr/drafts/` (heading exactly
   `# ADR-DRAFT: <Title>`, `Status: proposed`) and reference it from code **by
   path** (`docs/adr/drafts/<slug>.md`) so `adr promote` can rewrite the
   reference. Drafts are gitignored until `cargo xtask adr promote` runs at
   ship, so they land in the ship commit, not the PR diff.
4. **Redundant.** Comments that say only what the code already says. Delete.

## Scope

All first-party source, one standard everywhere — including tests:

- Crates/dirs: `common`, `server`, `storage`, `web`, `client`, `host`, `macros`,
  `xtask`, `tools`, `end2end`, `test-support`, `elisp`, `csr`.
- Root files: `Cargo.toml`, `clippy.toml`, `deny.toml`, `.rustfmt.toml`,
  `rust-toolchain.toml`, `flake.nix`.
- Also: `scripts/`, `.githooks/`, `.github/` workflow YAML.

Doc comments (`///`, `//!`) only where they exhibit the defects above; API doc
content itself is not under review. Out of scope: generated/vendored code,
`target/`, markdown docs and skills (documents, not code comments), and
**`storage/migrations/*.sql`** — sqlx checksums applied migrations by content,
so editing a comment there breaks existing databases.

## Protected comment patterns (must not delete, move, or rewrap)

The gate parses comments. Treat these as code, not prose:

- Marker family read by `cargo xtask check`: `cov:ignore` / `cov:ignore-start` /
  `cov:ignore-stop`, `crap:allow: <reason>`, `guard:allow`, `html-sink:allow`,
  `raw-html-door:allow`, `rendered-html-from-trusted:allow`, `wrapper:allow`,
  `// reason: …` on `#[apply(sqlite_only/postgres_only)]`,
  `// guard:no-backend — <reason>`. Per ADR-0094 a marker must sit **directly**
  above the reported line — no blank line or second comment may intervene — so
  deleting an adjacent comment or rewrapping near a marker can silently
  invalidate it (the gate fails closed; still, don't).
- Doc comments enforced by gates: ADR-0095 doctest-fence pairing (hidden lines
  must appear verbatim in a plain fence in the **same** doc comment — don't
  shorten or split those); the `csr/src/lib.rs` counterpart comment ADR-0109's
  xlang-literal gate reasons about.
- Comment text inside test fixture string literals (xtask step tests).
- `unreachable!("msg")` messages are required content, not comments.

## Deliverables

- **Direct edits** deleting/rewriting/shortening comments, committed in
  reviewable groups (by crate/area), Conventional Commit messages, each commit
  passing the pre-commit gate (full `cargo xtask check`). No behavior changes.
- **A findings report** cataloguing what was found and done, grouped by defect
  class with file:line references, including judgment calls and anything
  deliberately left alone. Authored on the branch in `docs/superpowers/plans/`,
  landed at ship as `docs/archive/2026-08-11-issue-930-comment-audit-report.md`
  (the durable home for dated snapshots).
- **ADR drafts** (authored autonomously) for promoted decisions, numbered at
  ship via `cargo xtask adr promote`.
- **A verdict on the standard itself.** A section of the findings report judging
  whether `CONTRIBUTING.md`'s "Comment for intent, not mechanics" is sufficient
  as written — informed by the defects actually found — and, if not, proposing
  concrete wording to make it clearer or more binding (e.g. naming the
  backward-looking and essay defect classes it currently doesn't forbid). Any
  resulting `CONTRIBUTING.md` edit is proposed in the report, not applied
  unilaterally.

## Non-goals

- No code changes beyond comments (and the mechanical knock-on of a comment edit
  — but never rewrapping adjacent to gate markers).
- No rewriting of correct, concise comments to taste; pure why-comments stay.
- No doc-comment coverage expansion; missing comments are not a defect here.

## Decisions (from interview)

- Edits land directly; the report is compiled alongside, not approved first.
- ADR drafts are authored autonomously; reviewed at ship (they cannot appear in
  the PR diff — gitignored until promote).
- Tests and tooling held to the same standard as production code.

## Risks

- Judgment calls on "load-bearing vs. noise": bias toward keeping intent and
  why, deleting narration. The report records every non-obvious call.
- Gate-marker adjacency (ADR-0094) is the sharpest edge; the gate fails closed,
  and every commit runs it.
- `csr` is effectively zero-yield, non-zero-risk (its one substantial comment is
  gate-load-bearing) — touch it only if a defect is clear.
