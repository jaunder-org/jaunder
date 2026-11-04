# Cached raw xtask Clippy

**Status: human-approved for implementation.** Design and implementation outline
are human-approved under requests `642deee8-e711-4467-9382-f6925a3135f4` and
`aec9cc61-7d0c-448b-b7ab-14f62060e1ef`. The design packet is
`issue1864-xtask-clippy-v1` (“Approve cached xtask spec”); the outline packet is
`issue1864-xtask-clippy-outline-v1` (“Approve outline and begin”). The separate
six-unit design approval is Manager request
`2f0929a1-ebbd-4735-9a6c-b53e99a7a141` / packet `issue1864-clippy-v2-coverage`,
exact response: “Approve six; design cached xtask”. This cached unit remains an
add-on, not a seventh unit in the approved six-unit scope. Proceed in the
admitted checkout within the approved scope. The Nix derivation runs raw Clippy
without invoking xtask or executing tests; normal host lifecycle execution
remains authorized. Merge requires separate approval.

## Outcome and boundary

Retain the existing required host check:

```text
cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings
```

Recommend that the new raw Nix unit be exposed in `nix flake check` and required
once by the existing Linux hermetic validation lane, alongside—but **not
inside**—the approved six-unit Clippy aggregate. This add-on is explicitly part
of this spec’s human approval ask, not part of the six-unit approval. Keep the
host check required; the Nix unit supplements it and does not substitute for it.

The Nix derivation runs raw Cargo Clippy against xtask source without invoking
the xtask CLI or executing tests. Cargo may execute build scripts and compile
procedural macros during compilation; `--all-targets` compiles selected test
targets but does not execute test binaries. Normal host lifecycle execution,
including hooks, focused tests, supported xtask validation/consumer proof, and
PR observation, remains authorized as applicable. ADR-0028 prohibits invoking
the xtask orchestrator from a derivation, not compiling or linting its source.
ADR-0146 currently keeps `xtask-clippy` host-only and excludes `xtask/` from the
flake source; introducing this Nix verdict needs narrow reconciliation of that
check/source boundary. No new ADR is proposed automatically.

## Derivation and inputs

Adapt existing `nix/xtask.nix`, rather than create a new packaging, vendoring,
native-dependency, or offline framework. It already uses `xtask/Cargo.lock`,
pinned-fork vendoring, native dependencies, and a package-specific source
closure. Preserve `JAUNDER_XTASK_BUILD_NIX_OUTPUT`: `xtask/build.rs` then
chooses a Nix-output build identity and returns before checkout Git
qualification. That identity is Nix-specific, not equivalent to host Git
provenance.

The bounded source-boundary delta is all-target Clippy versus the existing
binary package, which builds only `--bin xtask` with `doCheck = false`. Adapt or
verify its source predicate for the inputs required to compile all selected
targets, including known test-source includes: `.github/workflows/ci.yml`
(`xtask/src/gate.rs`), `.gitignore` (`xtask/src/production_baseline.rs`), and
JSON/JSONL fixtures under `xtask` testdata. These examples are known gaps, not
an exhaustive input census. Include the required `include_str!`/`env!` inputs,
manifests, Cargo configuration, toolchain, and local path-dependency closure.
Keep the xtask derivation separate from `static-code`; do not broaden unrelated
static-code inputs merely to force cache reuse.

## Consumer, caching, and exclusions

Expose the unit through `nix flake check` and require it once in the existing
Linux hermetic validation lane alongside, not inside, the six-unit aggregate.
This add-on is part of this spec’s approval ask, separate from the six-unit
approval. Use current static-check collection conventions and preserve existing
consumer/failure reporting and observations. Do not duplicate builds or
introduce a new aggregate API solely to schedule this unit; when requested
together, Nix can independently schedule derivation dependencies. This does not
freeze an in-progress #1859 interface or change hook/CI topology.

The additional unit may incur overlapping Clippy/dependency compilation. Its
independent derivation provides a cacheable boundary, but no speedup or reuse
benefit is measured. Keep Nix request duration and store-reuse classification
distinct from execution: record current Clippy execution only when positively
correlated to builder activity in the current request. Do not infer execution
from derivation identity, request duration, or historical logs. No forced
rebuilds or new timing protocol are proposed.

Do not change the six-unit population or aggregate, host Fix behavior, devtool
check catalog, or the required host `xtask-clippy` check. The prohibition on
executing xtask tests or its CLI applies only inside the Nix derivation; host
lifecycle tests and commands remain authorized as applicable. No unrelated
formatter, ERT, or TypeScript work is included.

## Acceptance criteria

1. Prove the derivation admits the compile-time inputs and all-target
   test-source fixtures required by Clippy. Test relevant source/config/tool
   invalidation and confirm unrelated inputs remain outside the unit’s **full**
   closure.
2. Reuse the xtask lock, pinned-fork vendor, and native dependencies; verify the
   derivation works with the Nix build-identity environment and without checkout
   Git qualification. Demonstrate unchanged valid local-store reuse; do not
   force rebuilds or purge caches.
3. Run the exact raw Clippy command in the derivation. A controlled lint failure
   must fail the unit and propagate through the designated Linux hermetic
   consumer. Restore or discard the failure fixture, then confirm clean inputs
   produce successful unit and consumer results. Confirm the Nix derivation does
   not execute tests or the xtask CLI.
4. Verify the flake and validation consumer request this verdict once, alongside
   (not within) the six-unit aggregate. Keep request duration, reuse, and
   current correlated execution evidence distinct.

**Approval record:** this cached raw xtask-Clippy add-on, including its
`nix flake check` exposure and one required request alongside the six-unit
aggregate in the Linux hermetic validation lane, has design approval and
separate implementation-outline approval as recorded above. Proceed within that
approved scope; merge still requires separate approval.

**Basis and limits:** Relevant paths: `nix/xtask.nix`, `xtask/build.rs`,
`xtask/Cargo.toml`, `xtask/src/steps/static_checks.rs`, ADR-0028, ADR-0146, and
`.xtask/evidence/xtask-clippy-feasibility.md`; cross-check:
`result:xtask-clippy-crosscheck#1`. No Nix build, Cargo metadata, Clippy run, or
source-filter probe is claimed.
