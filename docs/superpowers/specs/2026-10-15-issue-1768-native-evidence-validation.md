# Issue 1768: Native mutation evidence validation amendment

## Outcome

JAU-M33-1768-SPEC-002 materially amends approved JAU-M33-1768-SPEC-001. Keep
`.mutants-loop/verify.sh <package> <file>` as the documented entry point;
replace production jq validation with typed native validation and reporting.
Published reference: PR #1786, HEAD `7bf5283cef42a7285c4491c3df7e5b1b4230dfdd`,
tree `fe2462a2c8055ffdb46c883311768d072796ea06`. This draft is not approval.

## Load-bearing decisions

- Keep shell-owned producer execution through shared `run_mutants` unchanged,
  including coverage, filters, PostgreSQL selection, concurrency and isolation.
- After producer termination, the shell invokes exactly one new supported
  command:
  `cargo xtask mutants verify-evidence --package <package> --file <file> --output <artifact-directory> --producer-status <status>`.
  `--output` names the producer output root containing `mutants.out/`. The shell
  propagates the final native outcome, including launch failures.
- This explicitly replaces SPEC-001's no-new-interface exclusion for this
  command only. It adds no executable, generic validation framework or native
  runner.
- One focused Rust module inside existing xtask owns typed producer-evidence
  decoding, validation and successful counts/`STILL ALIVE` reporting. Its narrow
  interface takes explicit target, artifacts and producer status; it does not
  invoke the producer or duplicate shared invocation settings.
- Retain every original evidence guarantee: readable well-formed completed
  outcomes; successful baseline tests; applicable, nonempty examined mutants;
  consistent identities, phase results, category lists and counts; agreement
  between producer termination status and outcomes before authoritative output.
- Missing/malformed/incomplete/inconsistent evidence, failed baseline, tool
  errors, interruption, timeout holes and zero applicable mutants remain
  non-verification: explanatory diagnostics, nonzero exit and no authoritative
  counts or conclusion. Only agreeing producer statuses 0/2 can establish a
  valid result.
- Valid survivor findings retain actual counts, `STILL ALIVE` details and caller
  success; completed applicable zero-survivor runs retain successful zero-missed
  output. Native errors are nonzero. Remove `verify-outcomes.jq`, not its
  contract.
- Use existing serde/serde_json/anyhow for schema/error plumbing and Jiff for
  timestamp parsing, including fractional UTC seconds and chronological
  ordering; no handwritten calendar parser or other new dependency.
- Add direct xtask Jiff requirement `0.2.35`, matching the root requirement.
  Root lock resolves 0.2.35; xtask's separate lock already resolves compatible
  0.2.38 transitively. Retain that resolution: the intended lock delta is the
  xtask package's direct `jiff` edge, not a root-lock change or dependency
  upgrade. Review the actual generated delta; incompatible resolution needs
  reconciliation.
- Production uses the existing cargo-xtask launcher, then `nix run .#xtask` from
  the current checkout. Cold realization/build, source admission and each
  invocation's checkout preparation/hooks lifecycle remain real costs; warm runs
  reuse store outputs. Cost is unmeasured, not zero, and invocation count does
  not favor a full native runner: both designs add one xtask invocation after
  shell work.

## Acceptance

- Deterministic regressions execute real `verify.sh` and the real compiled xtask
  executable using the existing `CARGO_BIN_EXE_xtask` pattern. Fake cargo
  forwards only xtask delegation to that binary and substitutes only the
  mutation producer; run normal executable startup without admission overrides,
  fake validation or stale-code substitution. A non-Nix native binary proves
  behavior, not production Nix source admission.
- Existing auxiliary host tests prove all original invalid/error/zero-applicable
  cases and valid survivor, zero-survivor and mixed counts/details/exit behavior
  with isolated producer-shaped fixtures. Focused native-module tests
  complement, never replace, real-consumer proof; timestamp precision cases
  remain covered.
- Review command wiring, producer-only test forwarding, shared invocation
  parity, dependency delta and minimal local protocol guidance. Establish
  production checkout/source-admission evidence through targeted inspection and
  normal supported launcher, hook and CI paths, separately from native behavior
  proof; no new framework or campaign is required.
- Prior hooks/CI/readiness apply only to the published reference head. MERGE001
  stays held; redesign requires new focused proof, independent review, normal
  hooks, source-matching current CI and a new scoped human merge approval.

## Boundaries

No mutation matrix, VM campaign, runner redesign, coverage weakening,
suppression or external-skill edits. Original evidence policy is not reopened.
This new command and its admission/test-forwarding risk require a short outline
after spec approval; none is authorized or drafted now. No novel production
architecture decision is identified beyond existing host-tool/Jiff ADRs; surface
any genuine ADR need.
