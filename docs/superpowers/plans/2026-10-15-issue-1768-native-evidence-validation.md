# Issue 1768: Native evidence validation implementation outline

> Execute after outline approval with `jaunder-iterate`; use `jaunder-dispatch`
> for bounded tasks. Trigger: new command/interface, Nix admission and test
> forwarding.

## Scope

In: approved JAU-M33-1768-SPEC-002; preserve SPEC-001 behavior and shell
producer. Out: other interfaces/binaries, runner changes, production operations,
mutation matrix, VM campaign, suppressions, coverage weakening or external-skill
edits.

## Task outline

- [ ] 1. Native module validates typed evidence and reports valid findings.
  - Contract: explicit target, artifact root/status; complete, applicable,
    consistent evidence before counts/details. Add direct Jiff 0.2.35
    requirement and xtask lock edge, retaining locked 0.2.38; inspect generated
    delta, no root-lock change.
  - Verification: direct tests reject invalid evidence/status/baseline, timeout
    and zero-applicable cases; preserve valid counts/survivors and
    fractional-time order.
- [ ] 2. Smallest real-consumer tracer connects script and native reporting.
  - Contract: SPEC-002's `cargo xtask mutants verify-evidence` arguments; output
    root contains `mutants.out/`. Keep `common.sh`/`run_mutants` unchanged; one
    native call after producer termination, propagating final outcome/launch
    failure.
  - Verification: real `verify.sh`, `CARGO_BIN_EXE_xtask` normal startup and
    fake cargo forwarding only xtask/substituting only producer; one
    valid/invalid fixture.
- [ ] 3. Expand real-consumer regressions to the full original contract.
  - Contract: remove production jq/`verify-outcomes.jq`; minimally align
    protocol.
  - Verification: existing auxiliary host lane covers
    missing/malformed/incomplete/
    inconsistent/inapplicable/baseline/tool/timeout/zero evidence without
    authoritative output, plus survivor, zero-survivor and mixed
    counts/details/success exits.
- [ ] 4. Accept integrated redesign and establish fresh publication evidence.
  - Contract: independent review of wiring, shared invocation, dependencies and
    admission; targeted inspection plus supported launcher/hooks/CI prove
    production selection separately from native behavior. No new
    framework/campaign.
  - Verification: focused proof, normal commit/push hooks, current-head CI and
    final conformance; project enduring guidance, remove working contracts. Old
    green proves only head 7bf5283cef42a7285c4491c3df7e5b1b4230dfdd; MERGE001
    held; new scoped human merge approval required before land, never inferred
    here.

## Risk checks

No admission overrides, fake validation or stale-code fallback; admit tooling
inputs normally. Preserve time precision, filters/coverage/isolation and managed
checkout/ branch; one executor. Incompatible dependency/scope changes return to
Lead.
