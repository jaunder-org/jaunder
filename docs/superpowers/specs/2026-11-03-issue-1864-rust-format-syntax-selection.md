# Proposed #1864 scoped Rust-format verdict

> **Status: approved scoped-format spec; implementation outline separately
> approved.** The human approved the unchanged `web/src` + `csr/src` scope and
> coherent `fmt`/`tools-fmt`/`leptosfmt` Nix verdict under Manager request
> `1b68e607-ca9a-460b-b005-179532a44a9d` / issue `issue1864-rust-format-v3`,
> with the exact answer “Approve scoped-format spec.” Separately, the human
> approved the outline and authorized execution under request
> `9b7971b8-088c-4d0a-bbf4-8dd60bf14d40` / issue
> `issue1864-rust-format-outline-v1`, with the exact answer “Approve outline and
> begin.” This approval applies to this explicitly scoped execution in a
> separate checkout; merge remains separately gated.

## Outcome

Create one separately consumable Nix Rust-format verdict for existing product
`fmt`, tools `tools-fmt`, and `leptosfmt`. Scope leptosfmt to the known frontend
roots `web/src` and `csr/src`; preserve existing host Fix/index behavior and use
the same roots for host Verify/Fix and Nix Verify. No performance improvement is
claimed without measurement.

## Approach and tradeoffs

The current structural census found 217 unqualified source `view!` invocations
in 38 files, all within `web/src` and `csr/src`
(`.xtask/evidence/leptos-syntax-population.json`). This is source evidence, not
compiler expansion or active-cfg proof. It supports these existing directories
as the initial maintained Leptos input scope; it does not justify
scanning/parsing every Rust candidate across the repository. The earlier
anywhere-selector would read and parse all candidate sources even though it
formatted only selected files. Remove that selector/parser, its broad discovery
closure, and its added name/path/error plumbing from this design.

Invoke the existing pinned leptosfmt implementation on the two directory inputs,
retaining its default recognized macro paths `view` and `leptos::view` and their
full-path semantics. The upstream CLI recursively expands directory inputs. Do
not add a permanent AST audit, placement rejection guard, automatic
root-discovery rule, or policy forbidding Leptos syntax elsewhere. A future
frontend crate/root is added deliberately by updating formatter scope, its Nix
inputs, and maintained documentation through ordinary code review. Rustfmt
continues to cover Rust sources outside these Leptos roots; this design makes no
automatic Leptos-coverage claim for future roots until that update.

The pinned leptosfmt CLI has an observed failure-reporting limitation: per-file
read/parse errors and some glob iteration errors can be printed or filtered
while the command exits successfully
(`.xtask/evidence/leptosfmt-pinned-source.md`). Keep handling bounded to the two
scoped roots: use checked input enumeration with the same pinned glob semantics
and exclusion behavior before calling the unchanged formatter, and promote the
pinned tool's per-file error diagnostics to a failed named check. Do not add an
AST parser or generic discovery framework. Preserve the cooperative
no-racing-writers assumption. Focused implementation proof must establish
enumeration parity and error promotion before relying on the narrowed
invocation.

Keep product `fmt`, tools `tools-fmt --all`, and scoped `leptosfmt` in one
Rust-format Nix leaf: product/tools Cargo formatting populations overlap.
Preserve command identity/order and remove exactly these three invocations from
residual `static-code`, with no duplicate calls. Reuse existing devtool
definitions and the sandbox Cargo route. Declare the real closure, including
compiled `devtoolBin`, product/tools offline Cargo homes, pinned toolchain and
transitive source/vendor inputs; direct source filtering must not be described
as full closure independence. Directory scoping reduces Leptos formatter work
and inputs only to the extent the verified derivation closure permits; no
speedup or narrow-cache claim is made. Keep xtask's raw Cargo formatting as its
distinct approved leaf/spec, not part of this Rust-format design.

## Acceptance and boundaries

Verify host Fix/Verify and Nix Verify use only `web/src` and `csr/src` for
leptosfmt, with unchanged defaults and host Git/index admission. Test scoped
glob enumeration and fail-closed discovery/read/parse handling plus per-file
diagnostic promotion. Rust files outside these roots do not enter the Leptosfmt
invocation; files within the selected directories remain covered by directory
scope without per-file syntax selection. Verify `fmt`, `tools-fmt`, and
`leptosfmt` each run exactly once and are absent from residual Code. Consume
this Rust-format leaf exactly once in both `nix flake check` and hermetic
validation; demonstrate deliberate failure reaches the required consumer. Verify
relevant source/config/tool invalidation, valid local-store reuse, and
unrelated-input retention only where the complete transitive closure permits.
Report derivation identity, request duration, reuse, and existing
current-execution timing semantics without introducing a timing protocol. No
forced rebuild/cache purge.

Out of scope: repository-wide syntax selection, a permanent scope
guard/placement policy, raw xtask-format, Clippy, hook/CI routing, aggregate
API, generic framework, and a new ADR by default. Reconcile maintained docs and
applicable existing source-boundary decisions if implementation requires it.
Independent admitted checkouts need not wait for TypeScript/ERT work; coordinate
exact conflicting edits only and rebase actual conflicts. #1872's build-csr
design/Just pin remains human-approved (request
`e0d243bd-007d-40cc-b670-5e6e6fe35820`, packet `issue1859-build-csr-spec-v1`,
answer “Approve design and Just pin”); its separately owned execution is not a
dependency.

## Evidence

AST population and limitations: `.xtask/evidence/leptos-syntax-population.json`.
Pinned formatter defaults, CLI glob behavior, and per-file error limitation:
`.xtask/evidence/leptosfmt-pinned-source.md`. Cargo workspace populations and
formatter overlap: `.xtask/evidence/formatter-cargo-populations.json`. These are
feasibility evidence; implementation proof remains required as specified above.
