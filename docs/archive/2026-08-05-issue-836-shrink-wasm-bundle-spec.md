# Issue #836 — shrink the raw CSR wasm bundle

**Status:** draft, awaiting approval **Issue:**
[#836](https://github.com/jaunder-org/jaunder/issues/836) **Branch:**
`worktree-issue-836-shrink-wasm-bundle` (fork point tagged `wt-base-issue-836`)
**Predecessors:** #788 (observation) → #792 (corpus) → #794 (instrumentation) →
#818 (decomposition) → #840 (served-representation instrumentation; withdrew the
streaming claim)

## Why

#818 decomposed the firefox/chromium boot gap and found that firefox's wasm
compile+instantiate accounts for **80.5–87.6%** of it. SpiderMonkey's compile
_throughput_ is not ours to change; the **volume** it must compile is. The
bundle is **5 350 591 raw bytes**, and the measured relationship is roughly **88
ms of firefox compile per MiB of raw wasm**.

**Raw bytes are the target, not compressed bytes.** Brotli (862 755 bytes)
governs transfer; the wasm compiler's input is the decompressed artifact. Any
size work that optimises for the compressed figure is aimed at the wrong number.

#836 named two directions. **Direction 2 (verify streaming) is closed** by
merged PR [#840](https://github.com/jaunder-org/jaunder/pull/840): the
streaming-compilation claim was withdrawn, three candidate explanations were
tested and all three failed, and the residual is now recorded as unexplained.
This spec covers **direction 1 only**.

## Scope

Four workstreams, deliberately ordered (see _Sequencing_):

1. **Attribution** — make wasm bytes attributable to sections and crates.
2. **wasm-opt** — introduce an optimisation pass, which the build has never had.
3. **Layering** — move host-only code out of `common`, so the wasm graph cannot
   reach it.
4. **Budget** — gate the achieved size against regression in
   `cargo xtask validate`.

### Sequencing is load-bearing, not preference

Attribution lands **first**.

- `wasm-opt` drops the wasm name section by default, and that section is what
  per-crate attribution reads.
- Dead code is **already** substantially removed today — `wasm-ld` gc's
  unreachable sections, `lto = true` widens its view, and wasm-bindgen runs its
  own GC pass. A fat entry in `common/Cargo.toml` therefore does **not** imply
  fat bytes in the artifact. Without attribution, workstream 3 would be the
  cycle's largest diff spent on a guess.

Consequently **workstream 3 is data-gated** — see D0 for the threshold.

## Decisions

### D0 — "Material weight" is defined, not left to judgment

A dependency cluster is **material** if the per-crate rollup (D2) attributes **≥
25 KiB** of code-section bytes to the crates that cluster would remove, measured
on the attribution artifact (D1a) before any cuts land.

Clusters at or above the threshold are moved in this cycle. Clusters below it
are filed as deferred issues — the ADR-0058 layering argument for moving them
stands independently of bytes, but it is not this issue's business.

The same threshold governs whether `croner` (D7) and client log strings (D9) get
follow-up issues.

### D1 — Attribution is computed in-process by `xtask` via `wasmparser`

`xtask` is its own cargo workspace (`xtask/Cargo.toml:1`, root `Cargo.toml:14`
`exclude = ["xtask"]`), so a dependency added there cannot reach the main
workspace lock or the wasm graph. It already does host-side size analysis with
`flate2`/`brotli`, which is ADR-0028's stated reason for `audit-wasm` living in
xtask.

`devtool` computes nothing. Its only change in this cycle is running `wasm-opt`
(D4).

Rejected: `twiggy` as a devShell binary (adds a nix input, a shell restart, and
a parsed text contract to a lightly-maintained tool) and the `twiggy-*` crates
as a library (not a stable public API).

### D1a — Totals and breakdown are measured on _different artifacts_, and say so

These cannot be the same artifact: after D4 the shipped bundle has no name
section, so per-crate attribution of it is impossible. Conflating them would
make the breakdown unsatisfiable the moment wasm-opt lands.

| report                                  | artifact                                                                | why                                                                                |
| --------------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| totals (raw/gzip/brotli) and the budget | shipped `pkg/jaunder.wasm` from `nix build .#site`                      | what users download and what firefox compiles; unchanged from today's `audit-wasm` |
| section + per-crate breakdown           | `nix build .#csrWasm` → `lib/csr.wasm`, pre-wasm-bindgen and unstripped | still carries the name section, so crates are attributable                         |

The breakdown output states which artifact it describes and that its total is
**not** the shipped size. `--wasm <path>` allows pointing it at any wasm file.

Accepted imprecision: the attribution artifact predates wasm-bindgen and
wasm-opt, so its absolute bytes differ from the shipped bundle's. It is used to
_rank_ crates and choose targets, which is what D0 needs; it is not used for the
budget or for the reported delta.

### D2 — The breakdown reports sections _and_ per-crate, and must sum

The name section names only functions. Data segments and the type/import/element
sections are not attributable that way, so a per-crate function rollup alone
would silently explain only part of the file and invite misreading its
percentages as shares of the whole.

The report gives every section's byte size — **asserted by the tool to sum
exactly to the file size** — then a per-crate rollup _within_ the code section,
carrying an explicit `<unattributed>` bucket for unnamed functions. Percentages
are always stated against a named denominator.

### D3 — The shipped bundle carries no name section

`wasm-opt` discards names unless `-g` is passed, and that is the desired
outcome: the section is dead weight in production. Attribution reads the
separate unstripped artifact of D1a, so no data is lost.

Accepted cost: a browser stack trace from `console_error_panic_hook`
(`csr/src/lib.rs:57`) in the shipped bundle degrades to numeric offsets.

### D4 — `wasm-opt` enters `devtool csr-bundle`, at a level chosen by measurement

`tools/devtool/src/csr_bundle.rs:82` is the single implementation shared by the
host build (`cargo xtask build-csr`) and the Nix `csrWasmBundle` derivation, so
the _pipeline_ cannot drift between them. `wasm-opt` runs there, immediately
after `wasm-bindgen`. `binaryen` is added to `flake.nix` `nativeBuildInputs` for
`csrWasmBundle` and to the devShell.

`-O2`, `-Os` and `-Oz` are each measured on the real bundle and the winner
pinned by raw bytes. Expected winner `-Oz`. Runtime-performance regression from
aggressive size optimisation is acceptable: the Rust-side mount path measures
**1.7–12.7 ms**, invisible next to a ~470 ms compile.

### D4a — wasm-opt target features are passed explicitly

Binaryen rejects input using features it has not been told to allow, so an
unflagged `wasm-opt` can hard-fail the build — or, if features are instead
dropped, silently under-optimise.
`rustc --print cfg --target wasm32-unknown-unknown` reports the enabled set:
`bulk-memory`, `multivalue`, `mutable-globals`, `nontrapping-fptoint`,
`reference-types`, `sign-ext`. Each is passed as an explicit
`--enable-<feature>`.

Rejected: `-all`, which silently tracks whatever the installed binaryen
considers "all" and would let a binaryen upgrade change the accepted input set
without a diff.

`binaryen`'s version is pinned by `flake.lock` like every other tool; the
pipeline does not depend on a host-installed `wasm-opt`.

### D5 — `panic = "abort"` is rejected, permanently

`rustc --print cfg --target wasm32-unknown-unknown` reports `panic="abort"`
already. The lever yields **zero** wasm bytes. Setting it in the workspace
`[profile.release]` would additionally flip the **server's** release build from
unwind to abort, so a panicking tokio task would kill the process instead of
being isolated. Recorded so it is not re-proposed.

### D6 — Host-only code _moves to `host`_; no new feature flags

ADR-0058 (`docs/adr/0058-host-crate-layering.md:66`): _"Host-only shared
utilities have a clear home; future ones land in `host` rather than bloating
`common` or forcing cfg gates."_ ADR-0055 draws the wasm boundary at the module
level — _"not per line"_
(`docs/adr/0055-web-host-wasm-boundary-module-level.md:45`).

Four clusters have **zero** client consumers. Each is moved only if material
(D0):

| cluster     | what moves                                                                      | deps dropped from the wasm graph                   |
| ----------- | ------------------------------------------------------------------------------- | -------------------------------------------------- |
| syndication | `common/src/atompub/**`, `common/src/feed/{rss,atom}.rs`                        | `rss`, `atom_syndication`, `quick-xml`             |
| markup      | `mod sanitized` in `common/src/render.rs:240`                                   | `orgize`, `pulldown-cmark`, `ammonia`, `html5ever` |
| etag        | the sha2-based hashing constructor and `common/src/feed/metadata.rs::feed_etag` | `sha2`                                             |
| kdf         | `Password::{hash,verify}` (`common/src/password.rs:97,133`)                     | `argon2`                                           |

Feature-gating was considered and rejected: it would _add_ cfgs to escape the
very problem the project wants to escape, and it contradicts both ADRs above.

**Type stays, machinery leaves.** The etag and kdf clusters are method-level,
not module-level, so ADR-0055 does not justify them — **ADR-0058 does**
("host-only shared utilities land in `host`"). In both, the domain newtype
remains in `common` and only the host-only computation departs:

- `ETag` stays in `common`; `common/src/test_support.rs:17` `parse_etag` keeps
  working because parsing needs no `sha2`. Only hashing-based construction and
  `feed_etag` move. `host` already depends on `common` (`host/Cargo.toml:16`)
  and `storage` already depends on `host` (`storage/Cargo.toml:13`), so no new
  edges are needed.
- `Password`/`ProfferedPassword` and their `FromStr` policy check stay in
  `common` — the client validates against them per ADR-0065. Only
  `hash`/`verify` move, taking the `cheap-kdf` and `test-utils` features with
  them.

The markup move requires a minting door on `RenderedHtml` for `host`, and
deletes `storage/Cargo.toml:12`'s `common/sanitize` feature enablement.

### D7 — `croner` stays in the wasm graph

Confirmed client-reachable on two independent paths:
`web/src/backup/component.rs:90` constructs a `Field::<BackupSchedule>`, whose
`error_for` (`web/src/forms/field.rs:106`) parses on the wasm-only
`BackupSettingsPage` (routed at `web/src/app/component.rs:119`); and it is a
`#[server]` wire-arg type at `web/src/backup/api.rs:39` whose validating
`Deserialize` compiles into the wasm client stub.

Policy, not accident. ADR-0065
(`docs/adr/0065-client-side-domain-validation.md:24`): _"the newtype's `FromStr`
runs in the browser… validate on the client with the same function the server's
`Deserialize` routes through"_ — and it explicitly rejects re-implementing a
rule in `web` to keep it out of the bundle.

If `croner` is material (D0), that is a **separate design question** (a smaller
cron validator, or splitting syntactic from full validation) and is filed, not
solved here.

### D8 — The budget is a committed raw-byte ceiling with explicit headroom

The ceiling is a single `const` in `xtask` source with a doc comment stating the
achieved size, the headroom, and why the measurement is raw rather than
compressed. It is asserted in `cargo xtask validate` using the same measurement
`audit-wasm` already produces for the shipped artifact (D1a), so the two can
never disagree.

The failure message prints actual bytes, the ceiling, and directs the reader to
lower the number deliberately when a win lands.

A strict no-headroom ratchet was rejected: any innocent dependency bump turns
the gate red, the fix is always "raise the number", and the gate loses its
authority. Headroom is explicit and reviewable instead of implied.

`xtask/src/lib.rs:127`'s "This is a manual tool — it is not part of
`check`/`validate`" becomes false and is updated. No ADR amendment is needed:
ADR-0028 governs devtool-vs-xtask _placement_ only.

### D9 — Client log strings are measured, not muted

`csr/src/lib.rs:56` initialises `console_log` at `Level::Debug`, compiling every
`debug!`/`trace!` format string into the bundle. `log`'s `release_max_level_*`
features would strip them.

Not done here. #839 exists precisely to _start_ capturing browser console
output, because its absence is what made the streaming question unanswerable in
#840. Muting client logs in the same cycle works against that. Their weight is
reported; a follow-up is filed if material (D0).

### D10 — One ADR: the raw-size budget

Covering why the budget is on **raw** bytes rather than brotli, why headroom
exists, and how the ceiling is lowered. This is the decision a future reader
would otherwise reverse-engineer — every size instinct says "measure
compressed", and someone will eventually "fix" it.

Drafted numberless in `docs/adr/drafts/`, numbered at ship by
`cargo xtask adr promote`.

D4/D4a's pipeline change and D5's rejection are recorded in this spec and in
code comments rather than as ADRs — neither is contestable once explained.

### D11 — The boot capture is before-and-after on this tree, with a pre-registered prediction

A baseline capture is taken on this branch **before** the cuts and an
after-capture on the same protocol, both on the post-#840 tree. #818's corpus is
not reused as the baseline: it predates #840, so any difference would confound
the size change with everything else that landed.

The predicted compile-time drop is derived from 88 ms/MiB and **committed to the
repository** before the after-capture is run, so the ordering is checkable from
git ancestry rather than asserted. The corpus itself lives outside the repo, so
the committed write-up carries the protocol, capture counts and summary
statistics needed to review the claim without it.

### D12 — No size threshold gates shipping

Acceptance is measuring and recording honestly, not hitting a number.
Attribution decides which moves happen and wasm-opt's win is whatever it is, so
the delta is not knowable in advance. A hard floor would block a correct,
well-measured branch over an outcome we do not control, and invite byte-chasing
past the point of good judgment.

Note this is not in tension with D0: D0 is a threshold for _whether a cluster is
worth moving_, not for whether the branch may ship.

## Acceptance criteria

Each is checkable from the repository and the PR.

**Attribution**

- **A1** `cargo xtask audit-wasm` reports every wasm section's byte size for the
  attribution artifact, and fails if those sizes do not sum exactly to the file
  size.
- **A2** The same report gives a per-crate rollup within the code section,
  including an `<unattributed>` bucket, with percentages stated against a named
  denominator.
- **A3** The breakdown output names the artifact it describes and states that
  its total is not the shipped bundle size; `--wasm <path>` overrides the
  artifact.
- **A4** `--json` emits the section and per-crate data in machine-readable form.
- **A5** The breakdown is unit-tested against a checked-in fixture wasm,
  covering the sum-to-file-size assertion, the `<unattributed>` bucket, and a
  failing (non-summing) input.
- **A6** The pre-cut breakdown is recorded in the PR description and in
  `docs/observability.md`, including the per-cluster byte totals D0's threshold
  is applied to.

**wasm-opt**

- **A7** `devtool csr-bundle` runs `wasm-opt` after `wasm-bindgen`; `binaryen`
  is present in both `flake.nix` `nativeBuildInputs` for `csrWasmBundle` and the
  devShell.
- **A8** The `wasm-opt` argument vector is built by one function and
  unit-tested, asserting the pinned optimisation level and an explicit
  `--enable-` flag for each of `bulk-memory`, `multivalue`, `mutable-globals`,
  `nontrapping-fptoint`, `reference-types`, `sign-ext`, and no `-all`.
- **A9** Raw bytes for `-O2`, `-Os` and `-Oz` on the real bundle are recorded in
  the PR description, and the pinned level is the smallest of the three.
- **A10** The shipped `pkg/jaunder.wasm` contains no name section, verified by a
  test or a build-time assertion rather than by inspection.
- **A11** `cargo xtask build-csr` and the Nix `csrWasmBundle` derivation invoke
  `wasm-opt` through the same code path, so neither can gain or lose the pass
  independently.

**Layering**

- **A12** Every cluster in D6's table that D0 marks material is moved to `host`;
  every cluster below the threshold has a filed issue. The PR states, per
  cluster, the measured bytes and the resulting decision.
- **A13** No `#[cfg(feature = …)]` or `#[cfg(target_arch = …)]` is added to
  `common/src` by this work. (`#[cfg(test)]` is not in scope.)
- **A14** For each cluster moved, its feature flag and every one of its cfg
  sites are removed from `common`: `sanitize`
  (`common/src/render.rs:239,594,1133`) with markup, `cheap-kdf`/`test-utils`
  (`common/src/password.rs:109,116`, `common/src/lib.rs:60`) with kdf.
- **A15** If markup moves, `storage/Cargo.toml:12` no longer enables
  `common/sanitize`.
- **A16** `cargo tree -p csr --target wasm32-unknown-unknown` no longer lists
  the deps in the "deps dropped" column for each cluster moved.
- **A17** `ETag`, `Password` and `ProfferedPassword` remain in `common` with
  their parsing and `FromStr` policy checks intact, and
  `common/src/test_support.rs:17` `parse_etag` still compiles for wasm.
- **A18** `croner` remains reachable from client code; the `BackupSchedule`
  validation path in `web/src/backup/component.rs` is unchanged.
- **A19** The vestigial `croner` declaration at `web/Cargo.toml:32` is removed.
- **A20** `orgize` and `pulldown-cmark` are no longer non-optional dependencies
  of `common`: removed outright if markup moves, otherwise made `optional` and
  excluded from the wasm graph. (Today they are non-optional while their only
  users sit behind `sanitize`, which is never enabled for wasm.)

**Budget**

- **A21** `cargo xtask validate` fails when raw shipped `pkg/jaunder.wasm`
  exceeds the committed ceiling, and the failure message prints actual bytes,
  the ceiling, and how to lower it.
- **A22** The ceiling is a single `xtask` constant whose doc comment states the
  achieved size, the headroom, and why the budget is raw rather than compressed.
- **A23** A test proves the check fails on an over-ceiling input and passes on
  an under-ceiling one, without running a real `nix build`.
- **A24** The budget reads the same measurement `audit-wasm` produces for the
  shipped artifact, and `xtask/src/lib.rs:127`'s "not part of
  `check`/`validate`" comment no longer contradicts the code.

**Measurement**

- **A25** Before and after raw byte counts, both from `cargo xtask audit-wasm`,
  are recorded in the PR description and in `docs/observability.md`.
- **A26** A committed file states the predicted firefox compile-time drop,
  derived from 88 ms/MiB and the measured byte delta, and the commit adding it
  is an **ancestor** of the commit recording observed results.
- **A27** The write-up reports predicted vs observed, and carries the capture
  protocol, capture counts and summary statistics — enough to review the claim
  without the corpus. The corpus is preserved at
  `~/measurements/jaunder/issue-836-wasm-shrink/`.
- **A28** `docs/observability.md:642`'s "levers and the unverified streaming
  question are #836" line is updated to point at the outcome, so it no longer
  dangles.

**Records**

- **A29** A numberless ADR draft exists in `docs/adr/drafts/` covering D8 and
  D10.
- **A30** D5's rejection of `panic = "abort"` is recorded where someone would
  try it — a comment at the workspace `[profile.release]` in the root
  `Cargo.toml`.
- **A31** Client log-string weight (D9) is reported in the PR, with a filed
  issue if it exceeds D0's threshold.
- **A32** Each separable concern under _Deferred_ has a filed issue, referenced
  from this spec and from the PR.

## Deferred — filed as issues, not solved here

Resolved at ship — every bullet below now names its disposition.

- Any D6 cluster below D0's threshold; the ADR-0058 layering argument stands
  independently of bytes. **All four clusters measured at 0 bytes**, so all four
  are below threshold and none moved — filed on layering grounds alone as
  [#855](https://github.com/jaunder-org/jaunder/issues/855).
- `croner`'s bundle weight, if material — a smaller cron validator or a
  syntactic/full validation split (D7). **9.7 KiB, below threshold; no issue
  filed** — and ADR-0065 requires the client to validate through the server's
  own `FromStr`, so it stays.
- Client log-string weight, if material — `release_max_level_*` (D9), sequenced
  against #839. **~0 bytes (11, under the noise floor); no issue filed** — there
  are no `debug!`/`trace!` call sites in the wasm crates to strip.
- Splitting `common` into separate crates so the wasm graph cannot structurally
  reach host-only code. Raised and set aside as a much larger refactor with its
  own ADR — filed as [#847](https://github.com/jaunder-org/jaunder/issues/847).
- `serde_json` is 145 KiB of the code section, the only application-level entry
  in the top ten — found by the attribution built here, outside the original
  scope. Filed as [#856](https://github.com/jaunder-org/jaunder/issues/856).
- Firefox's wasm instantiate has a ~377 ms **size-independent** floor, uncovered
  by the three-arm capture and deliberately left unexplained — filed as
  [#864](https://github.com/jaunder-org/jaunder/issues/864).

## Out of scope

- Direction 2 of #836 (verify streaming) — closed by #840.
- The unexplained fetch/instantiate attribution asymmetry between engines —
  deliberately recorded as unexplained by #840; two hypotheses have already died
  there.
- #801's CSR mount cost — #818 showed the Rust mount path is 1.7–12.7 ms and is
  not where the time goes.
- Compressed (brotli/gzip) transfer size as an optimisation target.
- Byte-for-byte reproducibility between the host and Nix builds.
  `cargo xtask build-csr` is debug by default (`xtask/src/lib.rs:160`) while Nix
  builds release through crane, so the artifacts already differ. The property
  this cycle preserves is that both run the _same pipeline_ (A11), which is what
  `csr_bundle.rs` was created to guarantee.

## Operational note

Adding `binaryen` to the devShell requires a **shell restart** before the binary
is directly usable in this session. Plan tasks are sequenced so devShell changes
land early and the restart is requested once.
