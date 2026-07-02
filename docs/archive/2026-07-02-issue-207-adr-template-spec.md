# Spec — issue #207: canonical `docs/adr/template.md`

## Problem

Since #196 (PR#204) the `adr-format` gate (`xtask/src/adr_readme.rs`) enforces
two hard ADR-authoring invariants:

- line 1 is `# ADR-NNNN: <non-empty title>` (legacy `# NNNN. Title` rejected),
  and
- a `- Status: <token>` line with a token from
  `STATUS_VOCAB = [proposed, accepted, superseded, deprecated, rejected]`.

An author writing an ADR freehand discovers those rules by **failing the gate**.
There is no positive, in-repo artifact to start from. The `jaunder-adr` agent
skill embeds a template inline, but that copy lives in `~/.config/claude/` — it
can drift from the gate and humans writing ADRs never see it.

## Goal

Add an in-repo canonical ADR skeleton, `docs/adr/template.md`, as the positive
counterpart to the `adr-format` gate: **copy it and the first commit passes.**
It is the single source of truth the `jaunder-adr` skill will defer to (skill
repoint is a follow-up outside this repo).

## Decisions (resolved)

1. **Path / name: `docs/adr/template.md`.** Lives beside the ADRs it seeds.
2. **Gate-invisible by construction.** `ids::leading_number("template.md")` is
   `None`, so the file is skipped by all three numeric gates — `adr-format` and
   `adr-readme-parity` iterate only `NNNN-*.md`, and `identifier-collisions`
   groups by leading number. The template is therefore neither treated as an ADR
   nor projected into the README table. No gate code changes.
3. **Contents — mirror the house style (`docs/adr/0043-*`) and the skill's
   inline copy byte-for-byte** so the eventual skill repoint is a no-op swap:

   ```markdown
   # ADR-0000: <Title>

   - Status: proposed
   - Date: <YYYY-MM-DD>
   - Issue: [#<N>](https://github.com/jaunder-org/jaunder/issues/<N>)

   ## Context

   <the forces and constraints that make this a decision>

   ## Decision

   <what we're doing, stated so a future reader can't reverse it by accident>

   ## Consequences

   <what this commits us to; follow-ups; what it rules out>
   ```

   The `# ADR-0000:` heading means a fresh
   `cp docs/adr/template.md docs/adr/0000-<slug>.md` is already
   `adr-format`-valid (heading number matches the `0000` filename);
   `cargo xtask adr renumber` then rewrites `ADR-0000` → the assigned number.
   Placeholder body prose (`<…>`) is guidance, not gated.

4. **No new ADR.** The governing decision (always-0000 authoring + generated
   table) is already ADR-0036 and its #196 addendum; this is implementation, not
   a new choice.

## Open choices (recommended defaults — confirm or redirect at approval)

- **A. Link the template from `CONTRIBUTING.md`'s ADR section — RECOMMEND YES.**
  One line ("New ADR? Copy `docs/adr/template.md` …"). `CONTRIBUTING.md` is the
  definitive guide (per `CLAUDE.md`), so discoverability belongs there. Cheap;
  no downside.
- **B. Add a confirming xtask unit test that the gates ignore `template.md` —
  RECOMMEND YES.** A small test in `adr_readme.rs` asserting `format_problems` /
  `parity_report` return nothing for a `docs/adr/template.md` present on disk.
  Locks decision (2) against a future refactor that starts globbing
  `docs/adr/*.md`. Turns a pure-docs change into docs + one test; no production
  code changes.

## Acceptance criteria

1. `docs/adr/template.md` exists with the exact content in decision (3),
   prettier-clean.
2. The template is byte-identical to the `jaunder-adr` skill's current inline
   template (so the follow-up repoint is a literal swap).
3. `cargo xtask validate --no-e2e` is green — in particular `adr-format`,
   `adr-readme-parity`, and `identifier-collisions` still pass with
   `template.md` present (it is neither flagged nor added as a README row).
4. If **A** accepted: `CONTRIBUTING.md` has a one-line pointer to the template.
5. If **B** accepted: an xtask test asserts the gates ignore
   `docs/adr/template.md`, and it fails if that file were ever treated as an
   ADR.

## Out of scope

- Any gate keeping the template in sync with `STATUS_VOCAB` / the heading rule
  (deferred unless it actually drifts — per the #196 discussion).
- The `jaunder-adr` skill repoint (a `~/.config/claude/` edit, not this
  repo/PR).
- e2e-affecting surfaces (none touched; `--no-e2e` validate is the right gate).
