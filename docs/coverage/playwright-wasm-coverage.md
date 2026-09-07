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
cargo xtask wasm-coverage measure --quiescent-window "2026-09-07 user-confirmed idle host"
```

The latter completed with exit 0 in 1,094,165 ms. The retained primary evidence
is `.xtask/gcroots/wasm-coverage-csr/`; the executable browser and reconciled
evidence is `.xtask/wasm-coverage/`.

## Build, formats, and scope

The diagnostic producer used `nightly-2026-07-27`
(`rustc 1.99.0-nightly  dc3f85158`, LLVM 22.1.8),
clang/`llvm-profdata`/`llvm-cov` 22.1.8, `wasm-bindgen` 0.2.121, `wasm-opt` 132,
and minicov 0.3.8. Its pinned source is
`/nix/store/0ks4m16sz8wr5ijh17nssi51qqdxybkr-jaunder-site-cargo-source`; the
compiled prefix is `/build/wasm-coverage-csr/source`.

Only first-party crate `csr` was instrumented. First-party crates `client`,
`common`, `macros`, and `web` were omitted; this finding does **not** claim they
are covered. The instrumented build used `-Cinstrument-coverage`,
`-Zno-profiler-runtime`, `--emit=llvm-ir`, `-C link-arg=--no-gc-sections`, and
`-Zno-link`; pinned `rustc -Zlink-only` recreated Cargo's CSR/sysroot/minicov
link graph.

The retained formats are: raw LLVM instrumentation profiles
`profiles/browser.profraw`; merged LLVM profile data `mapped/browser.profdata`
and `merged/browser.profdata`; `llvm-cov` text line reports
(`mapped/llvm-cov.txt`, `merged/llvm-cov.txt`); versioned browser and aggregate
`status.json` payloads; the versioned measurement
`measurement/manifest-v1.json`; and the retained source identity and
source-mappable module relationship in the producer root.

`instrumented/csr.wasm` is the source-mappable module (SHA-256
`df8f4a9fbefe965e4cded0d6b7875ce012fa7febf7df51f6ac6c4e2f53b10436`). It is
linked from LLVM IR under the compiled prefix. The served derivative is
`pkg/jaunder.wasm` (SHA-256
`6b6a8f6c8a3126343cc7c04f99ece1ae485acebfcc6bbb0f0831e3b5fa7f062f`), the input's
wasm-bindgen/wasm-opt derivative—not the module used for Rust source mapping.

## Optimizer evidence and deviations

`coverage-metadata.json` proves both `__llvm_covfun` and `__llvm_covmap` exist
in the input and remain after wasm-bindgen and after wasm-opt
(`result: "preserved"`). Both processed modules retain `main`,
`__llvm_profile_runtime`, `jaunderCoverageModuleSignature`, and
`jaunderCoverageProfile`. wasm-bindgen metadata itself is absent after
wasm-bindgen; after wasm-opt it remains absent. That expected transformation is
distinct from preservation of the LLVM coverage sections.

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
signature `14804279403455803896` and on the served-module digest above.

| Browser  | CSR structural | Diagnostic export | Source mapping | Executed original Rust evidence                                                   |
| -------- | -------------- | ----------------- | -------------- | --------------------------------------------------------------------------------- |
| Chromium | passed         | passed            | passed         | `chromium/mapped/llvm-cov.txt`; count 1 on CSR lines 30–32, 50, 74, 77, and 79–84 |
| Firefox  | passed         | passed            | passed         | `firefox/mapped/llvm-cov.txt`; count 1 on CSR lines 30–32, 50, 74, 77, and 79–84  |

Each retains the same nonempty raw profile (SHA-256
`2e9eaaa48dd5e630a36fe8e9320fad14b4e0588781c1adcfc7c3a5dd86ec2a31`),
merged-per-browser profile data
(`d73b423927f9653049f0633628aba9c8e2310654f6781a92ad0d3e8c59077b41`), and mapped
report (`b7caf9015e273ee0509df1249f209d67f1ec972c7fec90985c33e8e4fb9b96ec`).

The Chromium and Firefox reports separately map executed original Rust lines in
the CSR entry module: for example each reports count 1 for lines 30–32
(`projector_seed`), line 50 (`mount`), and lines 74, 77, and 79–84 (`main`).
Source reconciliation uses the retained module/source identity and the stated
compiled-prefix equivalence; it is not a claim based on the served derivative's
post-optimization layout.

The aggregate `.xtask/wasm-coverage/status.json` is `v1`, contains exactly
Chromium and Firefox, has verdict `passed` and no blockers, and names
`.xtask/wasm-coverage/merged/{browser.profdata,llvm-cov.txt}`. Union is
conditional count summation, not a boolean OR: the separate browser count of 1
on representative lines 30–32, 50, 74, 77, and 79–84 becomes count 2 in
`merged/llvm-cov.txt`. A missing, stale, malformed, mismatched, partial, or
failed browser evidence set makes reconciliation fail closed and prevents the
merge; one browser cannot establish a green result for the other. The retained
producer contract also has deterministic injected export and mapping failure
cases, which retain status, diagnostics, and exact blockers rather than
synthesizing success.

## Quiescent paired measurement

The user-confirmed quiescent window was `2026-09-07 user-confirmed idle host`.
Four warm-ups were discarded (baseline and instrumented once in each browser).
The retained manifest then records five alternating baseline/instrumented pairs
per browser: 20 retained measured runs. Every run has a distinct cache-buster,
Nix realization, and retained run root under
`.xtask/wasm-coverage/measurement/runs/`, proving separate fresh realizations.
The metric is focused-flow milliseconds and raw uncompressed served
`pkg/jaunder.wasm` bytes.

| Browser  | Baseline median [range] | Instrumented median [range] |               Delta |
| -------- | ----------------------: | --------------------------: | ------------------: |
| Chromium |   1,084 ms [923, 1,099] |       1,073 ms [928, 1,117] | -11 ms (-1.014760%) |
| Firefox  | 2,263 ms [2,190, 2,353] |     2,201 ms [2,178, 2,236] | -62 ms (-2.739726%) |

Baseline served wasm was 2,633,716 bytes and instrumented served wasm was
2,663,925 bytes: +30,209 bytes (+1.147011%). The timing ranges overlap in both
browsers. Therefore this experiment makes **no measured slowdown claim and no
speedup claim**; the lower instrumented medians are not evidence of a speedup.

## Ownership, costs, and remaining risk

Nix owns reproducible diagnostic/baseline artifact production and the browser
execution. The host `xtask` owns artifact/status validation, source
reconciliation, conditional count union, and measurement orchestration. This
separation means host-side success is derived from retained evidence rather than
from an unrecorded browser result.

Remaining costs and risks are the 30,209-byte served-WASM overhead; the
1,094,165 ms quiescent-host experiment wall time; two browser/Nix realization
cost; continued compatibility of the pinned Rust/LLVM raw-profile, profdata, and
line-report formats; optimizer/bundler preservation; and the need to retain and
validate both complete browser evidence sets. A future permanent gate must keep
those costs and the both-browser fail-closed condition; dropping either browser
would not be supported by this finding.

## Evidence index

- Primary producer evidence:
  `.xtask/gcroots/wasm-coverage-csr/{status.json,build-configuration.json,coverage-metadata.json,source-identity.json,toolchain-identity.json}`.
- Browser evidence:
  `.xtask/wasm-coverage/{chromium,firefox}/{status.json,module/jaunder.wasm,profiles/browser.profraw,mapped/browser.profdata,mapped/llvm-cov.txt,diagnostics/}`.
- Reconciled evidence:
  `.xtask/wasm-coverage/{status.json,merged/browser.profdata,merged/llvm-cov.txt}`.
- Measurement evidence: `.xtask/wasm-coverage/measurement/manifest-v1.json` and
  its retained `warmups/` and `runs/` realizations.
