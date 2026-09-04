# Issue #948: Correct the flow-coverage namespace rationale

## Outcome

Current flow-coverage guidance will describe `code.namespace` honestly: it is
defensive corroboration for the unique current macro-derived span name and the
disambiguator for retained compatibility names that omit the vertical.

## Load-bearing decisions

- Retain the extractor's module check. A trace span counts only when both its
  candidate name and `code.namespace` identify the inventoried server function.
- State that `web.<vertical>.<ident>` is unique for valid current server
  functions because the macro enforces placement at `web/src/<vertical>/api.rs`;
  deeper server-function modules are rejected.
- For the current `web.<vertical>.<ident>` form, justify namespace matching as
  conservative corroboration that rejects foreign or malformed trace evidence
  without making the extractor depend indirectly on the macro's compile-time
  placement rule.
- Keep the extractor's supported historical and alternate candidate-name forms.
  Because `__server_<ident>` and bare `<ident>` omit the vertical,
  `code.namespace` remains their load-bearing disambiguator.
- Annotate ADR-0081 rather than rewriting its accepted historical decision.
- Apply the corrected rationale to the extractor's module documentation,
  candidate-name documentation, `identify` comment, focused test wording, and
  the user-facing observability guidance.

## Acceptance

- No current guidance claims that `posts::api` and `posts::api::listing` can
  both contain valid `#[macros::server]` functions.
- Current documentation distinguishes the unique current explicit name from
  compatibility names for which namespace remains the disambiguator.
- The extractor's `identify` comment and focused wrong-/missing-namespace test
  wording describe rejection of foreign or malformed evidence, not a second
  valid server function in an unreachable module.
- Existing wrong-namespace and missing-namespace behavior remains enforced by
  the focused extractor tests.
- Documentation and focused tests pass the repository's applicable checks.

## Boundaries

- Do not remove or weaken namespace matching.
- Do not change server-function placement, span naming, endpoint derivation, or
  flow-coverage artifact formats.
- Do not rewrite archived plans or other historical records.
- Do not add a new ADR; this change annotates the affected accepted ADR.
