# Playwright WASM coverage viability finding

## Verdict and scope

A permanent Rust/WASM coverage gate is **technically viable only as a
both-browser, fail-closed gate**: Chromium _and_ Firefox independently captured
an LLVM profile and mapped executed original Rust lines, and their profiles were
then count-summed. This is a feasibility finding, not a recommendation to land
such a gate. Issue #1281 adds no permanent CI gate, coverage denominator, or
production artifact change.

The successful executable commands were:

```bash
cargo xtask wasm-coverage probe
cargo xtask wasm-coverage measure --quiescent-window "2026-09-08 user-confirmed idle host"
```

The latter completed with exit 0 in 1,094,385 ms. The retained primary evidence
is `.xtask/gcroots/wasm-coverage-csr/`; the executable browser and reconciled
evidence is `.xtask/wasm-coverage/`.

## Build, formats, and scope

The diagnostic producer used `nightly-2026-07-27`
(`rustc 1.99.0-nightly  dc3f85158`, LLVM 22.1.8),
clang/`llvm-profdata`/`llvm-cov` 22.1.8, `wasm-bindgen` 0.2.121, `wasm-opt` 132,
and minicov 0.3.8. Its pinned source is
`/nix/store/fzpmflls2m9lwnsn629br7k36nh0r87y-jaunder-site-cargo-source`; the
compiled prefix is `/build/wasm-coverage-csr/source`.

Only first-party crate `csr` was instrumented. First-party crates `client`,
`common`, `macros`, and `web` were omitted; this finding does **not** claim they
are covered. The instrumented build used `-Cinstrument-coverage`,
`-Zno-profiler-runtime`, `--emit=llvm-ir`, `-C link-arg=--no-gc-sections`, and
`-Zno-link`; pinned `rustc -Zlink-only` recreated Cargo's CSR/sysroot/minicov
link graph.

The compiler and profile-tool semantics behind those flags are documented by
rustc's [instrumentation-based coverage guide][rust-coverage], including its
nightly profiler-runtime requirement, and the nightly-only
[`-Z no-profiler-runtime`][rust-no-profiler-runtime],
[`-Z no-link`][rust-no-link], and [`-Z link-only`][rust-link-only] flag
references. They explain the toolchain contract; the retained evidence cited
here establishes this experiment's actual outputs.

The retained formats are raw LLVM instrumentation profiles
`profiles/browser.profraw`; merged LLVM profile data `mapped/browser.profdata`
and `merged/browser.profdata`; `llvm-cov` text line reports
(`mapped/llvm-cov.txt`, `merged/llvm-cov.txt`); versioned browser and aggregate
`status.json` payloads; the versioned measurement
`measurement/manifest-v1.json`; and the retained source identity and
source-mappable module relationship in the producer root. LLVM's
[instrumentation-profile format][llvm-profile-format],
[`llvm-profdata merge`][llvm-profdata-merge], and [`llvm-cov` show/report/export
documentation][llvm-cov] define the respective raw-profile, merge/count, and
source-map/report/export tool semantics; the named files remain the evidence for
this run.

`instrumented/csr.wasm` is the source-mappable module (SHA-256
`895cbebb290f316dba812d187c081fbbdef57cfb0a291774847c26ce2f5a2950`). It is
linked from LLVM IR under the compiled prefix. The served derivative selected by
`pkg/manifest.json` is
`pkg/2a989f355bf9973891c40b7eac6f69e24da71309671ab0cd0e67e1beb354f578.wasm`; its
filename and SHA-256 are both
`2a989f355bf9973891c40b7eac6f69e24da71309671ab0cd0e67e1beb354f578`. It is the
input's wasm-bindgen/wasm-opt derivative—not the module used for Rust source
mapping.

## Optimizer evidence and deviations

`coverage-metadata.json` proves both `__llvm_covfun` and `__llvm_covmap` exist
in the input and remain after wasm-bindgen and after wasm-opt
(`result: "preserved"`). Both processed modules retain `main`,
`__llvm_profile_runtime`, `jaunderCoverageModuleSignature`, and
`jaunderCoverageProfile`. wasm-bindgen metadata itself is absent after
wasm-bindgen; after wasm-opt it remains absent. That expected transformation is
distinct from preservation of the LLVM coverage sections.

The documented roles of the [wasm-bindgen CLI][wasm-bindgen-cli] and [Binaryen
`wasm-opt`][binaryen-wasm-opt] describe the two transformations; the
metadata-preservation results above are observations from the retained
`coverage-metadata.json`, not claims inferred from those documents. The [minicov
0.3.8 API documentation][minicov-api] documents the profiler-runtime component
included in the diagnostic link graph.

The timing baseline intentionally used the same pinned nightly, source closure,
wasm-bindgen/wasm-opt bundle, service, and focused browser flow as the
diagnostic build. Its exact unavoidable deviations were: it omits
`-Cinstrument-coverage` and the minicov profiler runtime, and it omits the
`diagnostic-coverage` feature and diagnostic browser exports. The diagnostic
build is also a separately pinned diagnostic CSR derivative, rather than the
production CSR artifact; only `csr` is instrumented. These deviations bound the
experiment and must remain explicit in any future gate discussion.

## Browser evidence and source reconciliation

Both `.xtask/wasm-coverage/chromium/status.json` and
`.xtask/wasm-coverage/firefox/status.json` are `v1`, name the requested and
actual browser identically, and record `passed` for CSR structural validation,
diagnostic export, and source mapping with no blocker. They agree on module
signature `9457699261485885082` and on the content-addressed served-module
identity above.

[Playwright projects][playwright-projects] and its [browser-execution
documentation][playwright-browsers] define the configured multi-browser
execution mechanism. They do not establish the passed browser outcomes; the
versioned status files and retained profiles/reports below do.

| Browser  | CSR structural | Diagnostic export | Source mapping | Executed original Rust evidence                                              |
| -------- | -------------- | ----------------- | -------------- | ---------------------------------------------------------------------------- |
| Chromium | passed         | passed            | passed         | `chromium/mapped/llvm-cov.txt`; count 1 on CSR lines 30–32, 50–52, and 78–84 |
| Firefox  | passed         | passed            | passed         | `firefox/mapped/llvm-cov.txt`; count 1 on CSR lines 30–32, 50–52, and 78–84  |

Each retains the same nonempty raw profile (SHA-256
`b3cfdcac9875ab72827b9b18349d6edc65ebda3fa90169d95e9404b3a13b628c`),
merged-per-browser profile data
(`d78babe0750cc35453e56ea5184387319eeda5de88c7f31651db74aa3004af5b`), and mapped
report (`c12b8654237b7ef2b74d7de364333599c905a086397ab3eb08330ef878bfb8ff`).

The Chromium and Firefox reports separately map executed original Rust lines in
the CSR entry module: for example each reports count 1 for lines 30–32
(`projector_seed`), lines 50–52 (`mount`), and lines 73, 76, and 78–84 (`main`).
Source reconciliation uses the retained module and validates the retained
`nix-store-source` identity before mapping the compiled prefix to that immutable
source. It does not read mutable checkout sources or infer mapping from the
served derivative's post-optimization layout.

The aggregate `.xtask/wasm-coverage/status.json` is `v1`, contains exactly
Chromium and Firefox, has verdict `passed` and no blockers, and names
`.xtask/wasm-coverage/merged/{browser.profdata,llvm-cov.txt}`. Union is
conditional count summation, not a boolean OR: the separate browser count of 1
on representative lines 30–32, 50–52, 73, 76, and 78–84 becomes count 2 in
`merged/llvm-cov.txt`. A missing, stale, malformed, mismatched, partial, or
failed browser evidence set makes reconciliation fail closed and prevents the
merge; one browser cannot establish a green result for the other. The retained
producer contract also has deterministic injected export and mapping failure
cases, which retain status, diagnostics, and exact blockers rather than
synthesizing success.

## Quiescent paired measurement

The user-confirmed quiescent window was `2026-09-08 user-confirmed idle host`.
The measurement command first produced and validated fresh passing Chromium and
Firefox functional evidence. It then recorded five alternating
baseline/instrumented pairs per browser: 20 retained measured runs. Every
retained realization performed its own unmeasured mounted-CSR warm-up
immediately before the timed focused flow. Every run has a distinct
cache-buster, Nix realization, and retained run root under
`.xtask/wasm-coverage/measurement/runs/`, proving separate fresh realizations.
The metric is focused-flow milliseconds and raw uncompressed bytes of the
content-addressed wasm selected by each bundle's `pkg/manifest.json`.

| Browser  |   Baseline median [range] | Instrumented median [range] |                   Delta |
| -------- | ------------------------: | --------------------------: | ----------------------: |
| Chromium |         592 ms [532, 618] |           602 ms [576, 625] |     +10 ms (+1.689189%) |
| Firefox  | 11,199 ms [3,347, 15,303] |    4,586 ms [3,707, 30,367] | -6,613 ms (-59.049915%) |

The baseline served wasm was 3,333,996 bytes at
`pkg/01a228c4bb955c1f8056c5d66428cecc4f0c48d5d9c1c5834d725718b555841a.wasm`. The
instrumented served wasm was 3,365,744 bytes at the content-addressed path named
above: +31,748 bytes (+0.952251%). The timing ranges overlap in both browsers.
Therefore this experiment makes **no measured slowdown claim and no speedup
claim**; the median differences are not evidence of either.

## Ownership, costs, and remaining risk

Nix owns reproducible diagnostic/baseline artifact production and the browser
execution. The host `xtask` owns artifact/status validation, source
reconciliation, conditional count union, and measurement orchestration. This
separation means host-side success is derived from retained evidence rather than
from an unrecorded browser result.

This allocation of responsibilities is consistent with Nix's [derivation
model][nix-derivations] and its documented [`--impure` evaluation
option][nix-impure]: Nix realizes declared build inputs, while the host
orchestration validates the retained result. Those references define the
ownership model only; the concrete browser and reconciliation outcomes remain
local executable evidence.

Remaining costs and risks are the 31,748-byte served-WASM overhead; the
1,094,385 ms quiescent-host experiment wall time; two browser/Nix realization
cost; continued compatibility of the pinned Rust/LLVM raw-profile, profdata, and
line-report formats; optimizer/bundler preservation; and the need to retain and
validate both complete browser evidence sets. A future permanent gate must keep
those costs and the both-browser fail-closed condition; dropping either browser
would not be supported by this finding.

## Primary-source index

- Rust/rustc: [instrumentation-based coverage][rust-coverage],
  [`-Z no-profiler-runtime`][rust-no-profiler-runtime],
  [`-Z no-link`][rust-no-link], and [`-Z link-only`][rust-link-only].
- LLVM: [instrumentation-profile format][llvm-profile-format],
  [`llvm-profdata merge`][llvm-profdata-merge], and [`llvm-cov`
  show/report/export][llvm-cov].
- WASM processing/runtime: [wasm-bindgen CLI][wasm-bindgen-cli], [minicov 0.3.8
  API][minicov-api], and [Binaryen `wasm-opt`][binaryen-wasm-opt].
- Browser execution: [Playwright projects][playwright-projects] and [Playwright
  browser execution][playwright-browsers].
- Nix: [derivations][nix-derivations] and [the `--impure` evaluation
  option][nix-impure].

[rust-coverage]: https://doc.rust-lang.org/rustc/instrument-coverage.html
[rust-no-profiler-runtime]:
  https://doc.rust-lang.org/beta/unstable-book/compiler-flags/no-profiler-runtime.html
[rust-no-link]:
  https://doc.rust-lang.org/beta/unstable-book/compiler-flags/no-link.html
[rust-link-only]:
  https://doc.rust-lang.org/beta/unstable-book/compiler-flags/link-only.html
[llvm-profile-format]: https://llvm.org/docs/InstrProfileFormat.html
[llvm-profdata-merge]:
  https://llvm.org/docs/CommandGuide/llvm-profdata.html#profdata-merge
[llvm-cov]: https://llvm.org/docs/CommandGuide/llvm-cov.html
[wasm-bindgen-cli]:
  https://rustwasm.github.io/docs/wasm-bindgen/reference/cli.html
[minicov-api]: https://docs.rs/minicov/0.3.8/minicov/
[binaryen-wasm-opt]: https://github.com/WebAssembly/binaryen#wasm-opt
[playwright-projects]: https://playwright.dev/docs/test-projects
[playwright-browsers]: https://playwright.dev/docs/browsers
[nix-derivations]: https://nix.dev/manual/nix/stable/language/derivations.html
[nix-impure]:
  https://nix.dev/manual/nix/stable/command-ref/new-cli/nix.html#opt-impure

## Local executable-evidence index

- Primary producer evidence:
  `.xtask/gcroots/wasm-coverage-csr/{status.json,build-configuration.json,coverage-metadata.json,source-identity.json,toolchain-identity.json}`.
- Browser evidence:
  `.xtask/wasm-coverage/{chromium,firefox}/{status.json,module/pkg/2a989f355bf9973891c40b7eac6f69e24da71309671ab0cd0e67e1beb354f578.wasm,profiles/browser.profraw,mapped/browser.profdata,mapped/llvm-cov.txt,diagnostics/}`.
- Reconciled evidence:
  `.xtask/wasm-coverage/{status.json,merged/browser.profdata,merged/llvm-cov.txt}`.
- Measurement evidence: `.xtask/wasm-coverage/measurement/manifest-v1.json` and
  its retained `runs/` realizations.
