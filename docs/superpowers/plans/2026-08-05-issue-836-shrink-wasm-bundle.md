# Shrink the raw CSR wasm bundle — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-08-05-issue-836-shrink-wasm-bundle.md`](../specs/2026-08-05-issue-836-shrink-wasm-bundle.md)
— referenced by decision id (D0…D12) and acceptance criterion (A1…A32). **This
plan is "how"; the spec is "what/why."**

**Goal:** Cut the raw byte count of `pkg/jaunder.wasm` — the volume firefox must
compile — and gate the result against regression.

**Architecture:** Attribution lands first (`wasmparser` in `xtask`, measured on
a newly-exposed unstripped `.#csrWasm` artifact) because `wasm-opt` deletes the
name section it reads, and because existing dead-code elimination means a fat
dependency list does not imply fat bytes. The measured breakdown then decides
which `common` → `host` module moves are worth making (D0: ≥ 25 KiB). `wasm-opt`
enters the one shared bundle step. A committed ceiling in `xtask` locks the win
into `cargo xtask validate`.

**Tech Stack:** Rust (`xtask` — its own cargo workspace; `tools/devtool`;
`common`; `host`), `wasmparser` + `rustc-demangle` (new `xtask` deps),
`wasm-encoder` (new `xtask` dev-dep), binaryen's `wasm-opt`, Nix flake + crane,
Playwright e2e capture harness.

---

## Review header

**Scope — in:** attribution tooling in `audit-wasm`; `wasm-opt` in
`devtool csr-bundle`; data-gated `common` → `host` module moves; a raw-byte
budget in `validate`; before/after boot captures with a pre-registered
prediction; one ADR draft.

**Scope — out:** direction 2 of #836 (closed by #840); the unexplained
fetch/instantiate asymmetry; #801's mount cost; compressed-size optimisation;
host/Nix byte-for-byte reproducibility (`build-csr` is debug by default — spec
_Out of scope_).

**Tasks:**

1. File the separable concerns known now as issues.
2. Build-environment prerequisites: expose `.#csrWasm`, add `binaryen` — **shell
   restart**.
3. Section accounting (`wasmparser`), asserted to sum to file size.
4. Per-crate rollup from the name section, with `<unattributed>`.
5. Wire the breakdown into `cargo xtask audit-wasm` (`--breakdown`, `--wasm`,
   `--json`).
6. **Measure.** Record the pre-cut breakdown; apply D0's threshold; decide the
   clusters.
7. Baseline boot capture (before any cut).
8. `wasm-opt` in `devtool csr-bundle`, level chosen by measurement.
9. Assert the shipped bundle carries no name section.
10. Move the syndication cluster to `host` _(if material)_.
11. Move the markup cluster to `host` _(if material)_.
12. Move the etag cluster to `host` _(if material)_.
13. Move the kdf cluster to `host` _(if material)_.
14. Manifest hygiene: vestigial `croner`, `orgize`/`pulldown-cmark` optionality.
15. The raw-byte budget in `cargo xtask validate`.
16. ADR draft for the budget.
17. Commit the pre-registered prediction.
18. After-capture, write-up, and `docs/observability.md` updates.
19. File the remaining conditional issues.

**Key risks / decisions:**

- **Task 2 requires a shell restart** before `wasm-opt` is directly runnable.
  Sequenced early and requested once.
- **Tasks 10–13 may not all execute.** Task 6's measurement decides. A cluster
  below 25 KiB is filed (Task 19), not moved. Do not "just do them anyway" —
  that is the guess this plan is ordered to avoid.
- **`wasm-opt` can hard-fail the build** if target features are not allowed
  (D4a). Task 8 passes six explicit `--enable-` flags and unit-tests the
  argument vector.
- **Attribution artifact ≠ shipped artifact** (D1a). Its totals are for _ranking
  crates_, never for the budget or the reported delta. Every report states which
  file it describes.
- **Task 17 must be committed before Task 18 runs** — A26 is checked by git
  ancestry.

---

## Global Constraints

Copied from the spec; every task's requirements implicitly include these.

- **Raw bytes are the target**, never gzip/brotli (spec _Why_). Baseline: **5
  350 591 raw bytes**.
- **Materiality threshold (D0): ≥ 25 KiB** of attributed code-section bytes per
  cluster, measured on the Task 6 breakdown before any cuts land.
- **No new `#[cfg(feature = …)]` or `#[cfg(target_arch = …)]` in `common/src`**
  (A13). `#[cfg(test)]` is out of scope.
- **Types stay in `common`; host-only machinery leaves** (D6). `ETag`,
  `Password`, `ProfferedPassword` keep their parsing and `FromStr` policy
  checks.
- **`croner` stays in the wasm graph** (D7, ADR-0065). Do not touch
  `web/src/backup/component.rs`'s validation path.
- **`panic = "abort"` is rejected permanently** (D5). Do not add it.
- **wasm-opt target features are explicit**, never `-all` (D4a): `bulk-memory`,
  `multivalue`, `mutable-globals`, `nontrapping-fptoint`, `reference-types`,
  `sign-ext`.
- **Commits:** run `cargo xtask check` before each commit so the pre-commit gate
  passes clean (`jaunder-commit`). **No `Co-Authored-By` trailer.** Stage
  explicitly, then commit — never `git commit -- <paths>`.
- **`xtask` is its own cargo workspace** (`xtask/Cargo.toml:1`); its
  dependencies never reach the main workspace lock or the wasm graph.

---

## File Structure

| File                                                 | Responsibility                                                                                                                                                    |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xtask/src/wasm_sections.rs` **(create)**            | Pure: parse a wasm binary into per-section byte spans; assert they sum to file size.                                                                              |
| `xtask/src/wasm_symbols.rs` **(create)**             | Pure: read the name section into `(function name, body bytes)` pairs; demangle and roll up by crate, with `<unattributed>`.                                       |
| `xtask/src/audit_wasm.rs` **(modify)**               | Existing totals/budget path untouched; gains `breakdown()` + `render_breakdown()` composing the two modules above, and the artifact-provenance line.              |
| `xtask/src/wasm_budget.rs` **(create)**              | The ceiling constant and the pure over/under check.                                                                                                               |
| `xtask/src/steps/wasm_budget.rs` **(create)**        | The `validate` step: measure the shipped artifact, apply the check, produce a `StepResult`.                                                                       |
| `xtask/src/lib.rs` **(modify)**                      | CLI flags (`--breakdown`, `--wasm`), `validate` step wiring, doc-comment correction at `:127`.                                                                    |
| `xtask/src/result.rs` **(modify)**                   | Carry the breakdown report for `--json`/render.                                                                                                                   |
| `tools/devtool/src/csr_bundle.rs` **(modify)**       | Build the `wasm-opt` argument vector (pure, tested) and run the pass after `wasm-bindgen`.                                                                        |
| `flake.nix` **(modify)**                             | Expose `csrWasm` as a package; add `binaryen` to `csrWasmBundle` `nativeBuildInputs` and to the devShell.                                                         |
| `common/src/**` → `host/src/**` **(move)**           | The four D6 clusters, data-gated.                                                                                                                                 |
| `docs/adr/0102-wasm-raw-size-budget.md` **(create)** | D8/D10.                                                                                                                                                           |
| `docs/observability.md` **(modify)**                 | The #836 section: pre-cut breakdown and cluster verdicts, the pre-registered prediction, before/after bytes, predicted-vs-observed, and the dangling `:642` line. |
| `Cargo.toml` **(modify)**                            | A comment at `[profile.release]` recording why `panic = "abort"` is not set (A30).                                                                                |

---

## Task 1: File the separable concerns known now

Per `jaunder-start` step 5 — work that isn't this issue is filed up front, not
deferred to ship.

**Files:** none (GitHub only).

**Interfaces:**

- Produces: issue numbers, referenced by Task 19 and by the PR description
  (A32).

- [ ] **Step 1: File the `common` split issue**

Use `jaunder-issues`. Title and body:

> **title:**
> `common: split host-only code into its own crate so the wasm graph cannot reach it`
>
> `#836` moved four host-only clusters from `common` to `host` behind no feature
> flags (ADR-0058). That is a per-cluster remedy for a structural problem:
> `common` is a grab-bag that the wasm target compiles wholesale, so every
> future host-only addition is one review away from silently entering the bundle
> again.
>
> Considered during #836's design interview and set aside as a much larger
> refactor touching every importer, warranting its own ADR.
>
> Provenance: #818 (the finding), #836 (the per-cluster moves).

Labels: `test-infra`, plus whatever `jaunder-issues` prescribes for a refactor.
Milestone: none.

- [ ] **Step 2: Record the issue number**

Add it to the spec's _Deferred_ list as a link, replacing the bare bullet text
for the `common` split.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-08-05-issue-836-shrink-wasm-bundle.md
git commit -m "docs(spec): link the filed common-split issue (#836)"
```

---

## Task 2: Build-environment prerequisites

`nix build .#csrWasm` must work (Task 5 needs the unstripped artifact) and
`wasm-opt` must exist in both the Nix build and the devShell (Task 8).

**Files:**

- Modify: `flake.nix:1010` (packages), `flake.nix:452-455` (`csrWasmBundle`
  `nativeBuildInputs`), `flake.nix:1364` (`ciInputs`, where `wasm-bindgen-cli`
  already sits — **not** the `devOnly` list at `:1369-1382`)

**Interfaces:**

- Produces: flake attribute `csrWasm` → store path containing `lib/csr.wasm`;
  `wasm-opt` on PATH in `csrWasmBundle` and the devShell.

- [ ] **Step 1: Expose `csrWasm` as a package**

In `flake.nix`, in the `packages` attrset that already has `site = site;`
(`flake.nix:1010`), add — note `packages` is under
`pkgs.lib.optionalAttrs pkgs.stdenv.isLinux`, so `.#csrWasm` exists on Linux
only, which is where the audit runs:

```nix
            # The pre-wasm-bindgen, unstripped wasm. Exposed so `cargo xtask
            # audit-wasm --breakdown` has an artifact that still carries a name
            # section — `wasm-opt` strips names from the shipped bundle, so the
            # shipped file cannot be attributed to crates (#836, spec D1a).
            inherit csrWasm;
```

(`csrWasm` is a `let` binding at `flake.nix:428`, in the block closed by `in` at
`:1005`, so it is in scope here exactly as `site` is.)

- [ ] **Step 2: Add binaryen to the bundle derivation**

In `csrWasmBundle`'s `nativeBuildInputs` (`flake.nix:452-455`), add
`pkgs.binaryen` alongside `devtoolBin` and `wasm-bindgen-cli`.

- [ ] **Step 3: Add binaryen to the devShell**

Add `pkgs.binaryen` to the devShell package list (`flake.nix:1340-1382`), next
to `wasm-bindgen-cli`.

- [ ] **Step 4: Verify both**

Run: `nix build .#csrWasm --no-link --print-out-paths` Expected: a store path;
`ls <path>/lib/csr.wasm` exists.

Run: `nix develop -c wasm-opt --version` Expected: a binaryen version string.

- [ ] **Step 5: Commit**

```bash
git add flake.nix
git commit -m "build(nix): expose csrWasm, add binaryen to bundle and devShell (#836)"
```

- [ ] **Step 6: STOP — request a shell restart**

Tell the user the devShell changed and `wasm-opt` needs a restarted shell to be
directly runnable. Do not proceed until confirmed. (Spec _Operational note_.)

---

## Task 3: Section accounting

Every wasm section's on-disk byte span, asserted to sum to the file size (A1).
"On-disk span" = the section id byte + its LEB128 length prefix + payload, so
`8 + Σ spans == file length` (8 = magic + version).

**Files:**

- Create: `xtask/src/wasm_sections.rs`
- Modify: `xtask/Cargo.toml` (add `wasmparser = "0.244"` to `[dependencies]`,
  `wasm-encoder = "0.244"` to `[dev-dependencies]` — the versions already in the
  local cargo cache; let cargo resolve and commit the updated
  `xtask/Cargo.lock`), `xtask/src/lib.rs` (add `mod wasm_sections;` to the
  module list at `:24-51`)

**Interfaces:**

- Produces:
  ```rust
  pub struct SectionSize { pub name: String, pub bytes: u64 }
  pub fn section_sizes(wasm: &[u8]) -> anyhow::Result<Vec<SectionSize>>;
  ```
  `name` is the section's wasm name (`type`, `import`, `function`, `code`,
  `data`, …) or `custom:<id>` for custom sections (e.g. `custom:name`). Errs if
  the spans do not account for the whole file.
  ```rust
  /// The coverage invariant, separated so it can be tested without a wasm file:
  /// spans plus the 8-byte magic+version header must equal the file length.
  pub fn assert_spans_cover(file_len: u64, spans: &[SectionSize]) -> anyhow::Result<()>;
  ```

**Spec deviation, deliberate:** A5 says "a checked-in fixture wasm". These tests
synthesize fixtures with `wasm-encoder` instead — no binary blob in the tree,
and the fixture's shape is readable in the test. Note it in the PR description.

- [ ] **Step 1: Write the failing tests**

In `xtask/src/wasm_sections.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_encoder::{
        CodeSection, Function, FunctionSection, Instruction, Module, TypeSection, ValType,
    };

    /// A module with one function returning a constant, plus a custom section —
    /// enough to exercise type/function/code/custom spans.
    fn fixture() -> Vec<u8> {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut code = CodeSection::new();
        let mut f = Function::new([]);
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        m.section(&wasm_encoder::CustomSection {
            name: "producers".into(),
            data: std::borrow::Cow::Borrowed(b"xtask-test"),
        });
        m.finish()
    }

    #[test]
    fn spans_sum_to_the_file_size() {
        let wasm = fixture();
        let sizes = section_sizes(&wasm).unwrap();
        let total: u64 = sizes.iter().map(|s| s.bytes).sum();
        assert_eq!(
            total + 8,
            wasm.len() as u64,
            "sections plus the 8-byte header must account for the whole file"
        );
    }

    #[test]
    fn names_every_section_it_finds() {
        let sizes = section_sizes(&fixture()).unwrap();
        let names: Vec<&str> = sizes.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"type"), "{names:?}");
        assert!(names.contains(&"function"), "{names:?}");
        assert!(names.contains(&"code"), "{names:?}");
        assert!(
            names.contains(&"custom:producers"),
            "custom sections are named by their own name: {names:?}"
        );
    }

    #[test]
    fn every_span_is_non_zero() {
        for s in section_sizes(&fixture()).unwrap() {
            assert!(s.bytes > 0, "section {} has a zero span", s.name);
        }
    }

    #[test]
    fn spans_that_do_not_cover_the_file_are_rejected() {
        // A5's named invariant, pinned directly: `wasmparser` catches malformed
        // input, but the *coverage* check is ours, so it needs its own test rather
        // than riding on a parse failure.
        use super::assert_spans_cover;
        let spans = vec![SectionSize { name: "code".into(), bytes: 10 }];
        assert!(assert_spans_cover(100, &spans).is_err(), "under-coverage must Err");
        assert!(assert_spans_cover(1000, &spans).is_err(), "over-coverage must Err");
        assert!(assert_spans_cover(18, &spans).is_ok(), "10 + 8 header == 18");
    }

    #[test]
    fn rejects_a_truncated_module() {
        let mut wasm = fixture();
        wasm.truncate(wasm.len() - 3);
        assert!(
            section_sizes(&wasm).is_err(),
            "a truncated module must Err, not silently under-report"
        );
    }

    #[test]
    fn rejects_a_non_wasm_input() {
        assert!(section_sizes(b"not a wasm file at all").is_err());
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_sections` Expected: FAIL
— `section_sizes` / `SectionSize` not defined.

- [ ] **Step 3: Implement against the tests**

Write the bodies to the two signatures in **Interfaces**, using
`wasmparser::Parser::new(0).parse_all(wasm)`. `section_sizes` calls
`assert_spans_cover` before returning. Every branch is pinned by a test — the
sum invariant (both directions), custom-section naming, non-zero spans, the
truncated module, and the non-wasm input — so implement to satisfy them.

One detail the tests cannot express: a section's on-disk span is not its payload
length. Derive it from the payload's `range()` plus the id and LEB length prefix
bytes that precede it, i.e. take each `Payload`'s reported range and extend it
backwards to the section header; the sum invariant test is what proves you got
that arithmetic right.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_sections` Expected: PASS
(6 tests).

- [ ] **Step 5: Commit**

```bash
git add xtask/Cargo.toml xtask/Cargo.lock xtask/src/wasm_sections.rs xtask/src/lib.rs
git commit -m "feat(xtask): per-section wasm byte accounting that must sum (#836)"
```

---

## Task 4: Per-crate rollup

Attribute code-section function bodies to crates via the name section, with an
explicit `<unattributed>` bucket (A2). Parsing and rollup are separate so the
rollup is testable without any wasm at all.

**Files:**

- Create: `xtask/src/wasm_symbols.rs`
- Modify: `xtask/Cargo.toml` (add `rustc-demangle = "0.1"`), `xtask/src/lib.rs`
  (add `mod wasm_symbols;` to the module list at `:24-51`)

**Interfaces:**

- Consumes: nothing from Task 3 (deliberately independent).
- Produces:

  ```rust
  pub struct FunctionSize { pub name: Option<String>, pub bytes: u64 }
  pub struct CrateBytes { pub krate: String, pub bytes: u64 }
  /// Function body spans in the code section, paired with their name-section name.
  pub fn function_sizes(wasm: &[u8]) -> anyhow::Result<Vec<FunctionSize>>;
  /// Demangle and bucket by originating crate. Unnamed functions land in
  /// `UNATTRIBUTED`. Sorted by bytes descending.
  pub fn rollup(functions: &[FunctionSize]) -> Vec<CrateBytes>;
  pub const UNATTRIBUTED: &str = "<unattributed>";

  /// Shared test fixtures. Declared here in Task 4 so Task 9 can consume them
  /// without retro-editing this file.
  #[cfg(test)]
  pub mod tests_support {
      pub fn named_module() -> Vec<u8>;   // one function, with a name section
      pub fn unnamed_module() -> Vec<u8>; // the same module, name section omitted
  }
  ```

- [ ] **Step 1: Write the failing tests**

In `xtask/src/wasm_symbols.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: Option<&str>, bytes: u64) -> FunctionSize {
        FunctionSize { name: name.map(str::to_string), bytes }
    }

    #[test]
    fn buckets_legacy_mangled_names_by_crate() {
        let fns = vec![
            f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 100),
            f(Some("_ZN6orgize6render17hfedcba9876543210E"), 50),
            f(Some("_ZN4core3fmt5write17h1111111111111111E"), 30),
        ];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, "orgize");
        assert_eq!(r[0].bytes, 150);
        assert_eq!(r[1].krate, "core");
        assert_eq!(r[1].bytes, 30);
    }

    #[test]
    fn buckets_v0_mangled_names_by_crate() {
        // v0 mangling: _R + Nv + crate-name-with-length prefix
        let fns = vec![f(Some("_RNvCs1234_6croner5parse"), 64)];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, "croner", "v0 names must attribute too: {r:?}");
    }

    #[test]
    fn unnamed_functions_land_in_the_unattributed_bucket() {
        let fns = vec![f(None, 200), f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 100)];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, UNATTRIBUTED);
        assert_eq!(r[0].bytes, 200);
    }

    #[test]
    fn unmangled_names_are_unattributed_not_treated_as_crates() {
        // wasm-bindgen emits plain JS-glue shims with unmangled names; they are
        // real bytes but belong to no crate, and must not invent one.
        let fns = vec![f(Some("__wbindgen_malloc"), 40)];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, UNATTRIBUTED);
        assert_eq!(r[0].bytes, 40);
    }

    #[test]
    fn rollup_is_sorted_by_bytes_descending_and_conserves_total() {
        let fns = vec![
            f(Some("_ZN4core3fmt5write17h1111111111111111E"), 10),
            f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 500),
            f(None, 90),
        ];
        let r = rollup(&fns);
        assert!(r.windows(2).all(|w| w[0].bytes >= w[1].bytes), "{r:?}");
        assert_eq!(
            r.iter().map(|c| c.bytes).sum::<u64>(),
            600,
            "rollup must conserve every byte it was given"
        );
    }

    #[test]
    fn rollup_of_nothing_is_empty() {
        assert!(rollup(&[]).is_empty());
    }
}
```

Plus the shared fixture builders and one parsing test. Write `tests_support` as
a `#[cfg(test)] pub mod` at file scope (not inside `mod tests`), so Task 9 can
reach it:

```rust
#[cfg(test)]
pub mod tests_support {
    use wasm_encoder::{
        CodeSection, Function, FunctionSection, Instruction, Module, NameMap, NameSection,
        TypeSection, ValType,
    };

    fn base() -> Module {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut code = CodeSection::new();
        let mut fun = Function::new([]);
        fun.instruction(&Instruction::I32Const(7));
        fun.instruction(&Instruction::End);
        code.function(&fun);
        m.section(&code);
        m
    }

    pub const FIXTURE_FN: &str = "_ZN6orgize5parse17h0123456789abcdefE";

    pub fn named_module() -> Vec<u8> {
        let mut m = base();
        let mut names = NameSection::new();
        let mut map = NameMap::new();
        map.append(0, FIXTURE_FN);
        names.functions(&map);
        m.section(&names);
        m.finish()
    }

    pub fn unnamed_module() -> Vec<u8> {
        base().finish()
    }
}
```

and in `mod tests`:

```rust
    #[test]
    fn function_sizes_reads_names_from_the_name_section() {
        let got = function_sizes(&tests_support::named_module()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name.as_deref(), Some(tests_support::FIXTURE_FN));
        assert!(got[0].bytes > 0);
    }

    #[test]
    fn function_sizes_yields_unnamed_entries_without_a_name_section() {
        let got = function_sizes(&tests_support::unnamed_module()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, None, "no name section => no name, but still a body");
        assert!(got[0].bytes > 0);
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_symbols` Expected: FAIL —
`function_sizes` / `rollup` / `UNATTRIBUTED` not defined.

- [ ] **Step 3: Implement against the tests**

Signatures as in **Interfaces**. `function_sizes` walks the code section for
body spans and the `name` custom section's function-name subsection, pairing
them by function index. `rollup` demangles with `rustc_demangle::try_demangle`
and takes the first path segment as the crate; anything that fails to demangle
goes to `UNATTRIBUTED`.

Every branch is pinned: legacy mangling, v0 mangling, unnamed,
unmangled-but-named, ordering, byte conservation, and the empty case.

One invariant the tests state but the implementation must be written to hold
deliberately: **`rollup` conserves bytes** — no function may be dropped, only
bucketed. Any `filter` in that function is a bug.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_symbols` Expected: PASS
(8 tests).

- [ ] **Step 5: Commit**

```bash
git add xtask/Cargo.toml xtask/Cargo.lock xtask/src/wasm_symbols.rs xtask/src/lib.rs
git commit -m "feat(xtask): attribute wasm function bytes to crates (#836)"
```

---

## Task 5: Wire the breakdown into `audit-wasm`

**Files:**

- Modify: `xtask/src/audit_wasm.rs`, `xtask/src/lib.rs:120-136` (flags + doc
  comment), `xtask/src/result.rs:62`
- Test: in-file `#[cfg(test)]` in `xtask/src/audit_wasm.rs` (crate convention)

**Interfaces:**

- Consumes: `wasm_sections::section_sizes`,
  `wasm_symbols::{function_sizes, rollup, UNATTRIBUTED}`.
- Produces:

  ```rust
  #[derive(Debug, Serialize)]
  pub struct BreakdownReport {
      pub artifact: String,
      pub total_bytes: u64,
      pub sections: Vec<crate::wasm_sections::SectionSize>,
      pub code_bytes: u64,
      pub crates: Vec<crate::wasm_symbols::CrateBytes>,
  }
  pub fn breakdown(wasm_path: Option<&str>) -> anyhow::Result<BreakdownReport>;
  pub fn render_breakdown(report: &BreakdownReport) -> String;
  ```

  `wasm_path` `None` → `nix_build::build_out_path("csrWasm")` + `/lib/csr.wasm`.

- [ ] **Step 1: Write the failing tests**

Append to `xtask/src/audit_wasm.rs`'s `mod tests`:

```rust
    fn breakdown_fixture() -> BreakdownReport {
        use crate::wasm_sections::SectionSize;
        use crate::wasm_symbols::{CrateBytes, UNATTRIBUTED};
        BreakdownReport {
            artifact: "/nix/store/x-csr-wasm/lib/csr.wasm".into(),
            total_bytes: 5_350_591,
            sections: vec![
                SectionSize { name: "code".into(), bytes: 4_000_000 },
                SectionSize { name: "data".into(), bytes: 1_000_000 },
                SectionSize { name: "custom:name".into(), bytes: 350_591 },
            ],
            code_bytes: 4_000_000,
            crates: vec![
                CrateBytes { krate: "orgize".into(), bytes: 2_000_000 },
                CrateBytes { krate: UNATTRIBUTED.into(), bytes: 1_500_000 },
                CrateBytes { krate: "core".into(), bytes: 500_000 },
            ],
        }
    }

    #[test]
    fn render_breakdown_names_the_artifact_and_disclaims_shipped_size() {
        let t = render_breakdown(&breakdown_fixture());
        assert!(t.contains("/nix/store/x-csr-wasm/lib/csr.wasm"), "{t}");
        assert!(
            t.to_lowercase().contains("not the shipped"),
            "must state its total is not the shipped bundle size (spec D1a): {t}"
        );
    }

    #[test]
    fn render_breakdown_states_percentages_against_a_named_denominator() {
        let t = render_breakdown(&breakdown_fixture());
        // orgize is 2 MiB of the 4 MiB code section => 50%, denominated on `code`,
        // NOT on the 5.1 MiB file.
        assert!(t.contains("50.0%"), "{t}");
        assert!(
            t.contains("code section"),
            "the denominator must be named in the output: {t}"
        );
    }

    #[test]
    fn render_breakdown_shows_every_section_and_the_unattributed_bucket() {
        let t = render_breakdown(&breakdown_fixture());
        for s in ["code", "data", "custom:name"] {
            assert!(t.contains(s), "missing section {s}: {t}");
        }
        assert!(t.contains(crate::wasm_symbols::UNATTRIBUTED), "{t}");
    }

    #[test]
    fn breakdown_errors_when_the_artifact_is_missing() {
        let missing = "/nonexistent/csr.wasm";
        let err = breakdown(Some(missing)).unwrap_err().to_string();
        assert!(err.contains("csr.wasm"), "error names the artifact: {err}");
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml audit_wasm` Expected: FAIL —
`BreakdownReport` / `breakdown` / `render_breakdown` not defined.

- [ ] **Step 3: Implement against the tests**

Add the type and both functions to `audit_wasm.rs` per **Interfaces**.
`breakdown` reads the file, calls `section_sizes` and `function_sizes` +
`rollup`, and sets `code_bytes` from the `code` section's span.

`render_breakdown`'s content is pinned by the tests: artifact path, the "not the
shipped bundle size" disclaimer, per-section rows, per-crate rows with
percentages denominated on the named code section, and the `<unattributed>`
bucket. Follow `render_table`'s existing column style (`audit_wasm.rs:73`).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml audit_wasm` Expected: PASS.

- [ ] **Step 5: Wire the CLI**

In `xtask/src/lib.rs`, extend `Command::AuditWasm` (`:132-136`):

```rust
        /// Report per-section and per-crate byte attribution instead of totals.
        ///
        /// Measured on the pre-wasm-bindgen, unstripped `.#csrWasm` artifact,
        /// which still carries a name section — `wasm-opt` strips names from the
        /// shipped bundle, so the shipped file cannot be attributed (#836).
        /// Its total is NOT the shipped bundle size.
        #[arg(long)]
        breakdown: bool,
        /// Break down this wasm file instead of building `.#csrWasm`.
        #[arg(long, requires = "breakdown")]
        wasm: Option<String>,
```

Dispatch at `:511-526`: when `breakdown` is set, call
`audit_wasm::breakdown(wasm.as_deref())` and store the report on `CommandResult`
(add a `pub breakdown: Option<BreakdownReport>` field beside `pub audit` at
`result.rs:62`, rendered via `render_breakdown` and serialized for `--json`).

Also update the `:127` doc line — it stays accurate for now ("manual tool") and
is corrected in Task 15, so leave it; add the `--breakdown` example to
`after_help`.

- [ ] **Step 6: Verify end-to-end**

Run: `cargo xtask audit-wasm --breakdown` Expected: a section table summing to
the file size, a per-crate table, the artifact path, and the "not the shipped
bundle size" line.

Run: `cargo xtask --json audit-wasm --breakdown` Expected: JSON containing
`sections` and `crates` arrays (A4).

- [ ] **Step 7: Commit**

```bash
git add xtask/src/audit_wasm.rs xtask/src/lib.rs xtask/src/result.rs
git commit -m "feat(xtask): audit-wasm --breakdown, per-section and per-crate (#836)"
```

---

## Task 6: Measure — the pre-cut breakdown and the cluster decisions

This is the task the plan is ordered around. **No cuts have landed yet.** (A6,
D0.)

**Files:**

- Modify: `docs/observability.md`

**Interfaces:**

- Produces: the per-cluster byte totals and the material/not-material decision
  for each of the four D6 clusters. Tasks 10–13 and 19 consume these.

- [ ] **Step 1: Capture the breakdown**

Run: `cargo xtask --json audit-wasm --breakdown > /tmp/issue-836-breakdown.json`
Run: `cargo xtask audit-wasm --breakdown` Run: `cargo xtask audit-wasm` (totals
— confirm the 5 350 591 baseline still holds)

- [ ] **Step 2: Compute per-cluster totals**

Sum the rollup's `bytes` over each cluster's crates:

| cluster               | crates to sum                                      |
| --------------------- | -------------------------------------------------- |
| syndication           | `rss`, `atom_syndication`, `quick_xml`             |
| markup                | `orgize`, `pulldown_cmark`, `ammonia`, `html5ever` |
| etag                  | `sha2`                                             |
| kdf                   | `argon2`                                           |
| _(reference)_ croner  | `croner`                                           |
| _(reference)_ logging | `log`, `console_log`                               |

Note crate names appear demangled with underscores (`quick_xml`, not
`quick-xml`).

- [ ] **Step 3: Apply D0**

For each cluster: **≥ 25 KiB → material** (move it, Tasks 10–13). **< 25 KiB →
not material** (file it, Task 19). Write the number and the verdict down for
every row, including the two reference rows, which decide whether Task 19 files
croner (D7) and log-string (D9) follow-ups.

- [ ] **Step 4: Record it**

Add a `#836` section to `docs/observability.md` containing: the baseline
raw/gzip/brotli totals, the top ~15 rows of the per-crate rollup, the section
table, the per-cluster totals from Step 2, and the Step 3 verdicts. State
plainly that the rollup is measured on `.#csrWasm` and its total is not the
shipped size.

- [ ] **Step 5: Commit**

```bash
git add docs/observability.md
git commit -m "docs(observability): pre-cut wasm breakdown and cluster verdicts (#836)"
```

- [ ] **Step 6: Report the verdicts to the user**

State which of Tasks 10–13 will execute and which become filed issues, with the
numbers. This changes the remaining plan, so it is said out loud rather than
silently applied.

---

## Task 7: Baseline boot capture

Before any cut lands (D11). Reuse the #818 protocol so before and after are
comparable.

**Files:**

- Create: `~/measurements/jaunder/issue-836-wasm-shrink/before/` (outside the
  repo)

**Interfaces:**

- Produces: the "before" corpus and its summary statistics, consumed by Task 18.

- [ ] **Step 1: Confirm the protocol**

Read `docs/observability.md`'s #818 section for the exact capture protocol
(capture count, backends, browsers, worker settings). Record the protocol
verbatim in the corpus directory as `protocol.md` so Task 18 can repeat it
exactly.

- [ ] **Step 2: Run the baseline capture**

Run the #818 capture procedure on the current HEAD, writing to
`~/measurements/jaunder/issue-836-wasm-shrink/before/`.

- [ ] **Step 3: Summarize**

Produce the median wasm fetch / compile / instantiate figures per browser and
population, in the same shape as #818's table. Save as `summary.md` in the
corpus directory.

- [ ] **Step 4: Report**

State the baseline firefox compile figure to the user; Task 17's prediction is
derived from it.

_(No commit — the corpus lives outside the repo. Its statistics enter the repo
in Task 18, per A27.)_

---

## Task 8: `wasm-opt` in `devtool csr-bundle`

**Files:**

- Modify: `tools/devtool/src/csr_bundle.rs:82-113`
- Test: in-file `#[cfg(test)]` in `tools/devtool/src/csr_bundle.rs`

**Interfaces:**

- Produces:

  ```rust
  /// The `wasm-opt` argument vector: optimisation level, explicit target-feature
  /// enables, input and output paths.
  fn wasm_opt_args(level: &str, input: &Path, output: &Path) -> Vec<String>;
  /// Pinned by measurement in Step 6. Its doc comment records all three measured
  /// raw sizes (-O2 / -Os / -Oz) so the choice is auditable in the source, not
  /// only in the PR description.
  const WASM_OPT_LEVEL: &str = "-Oz";
  const WASM_TARGET_FEATURES: [&str; 6] = [
      "bulk-memory", "multivalue", "mutable-globals",
      "nontrapping-fptoint", "reference-types", "sign-ext",
  ];
  ```

- [ ] **Step 1: Write the failing tests**

Append to `csr_bundle.rs`'s `mod tests`:

```rust
    #[test]
    fn wasm_opt_args_carry_the_pinned_level() {
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(args.contains(&WASM_OPT_LEVEL.to_string()), "{args:?}");
    }

    #[test]
    fn wasm_opt_args_enable_every_rustc_target_feature() {
        // Binaryen rejects input using features it was not told to allow, so an
        // unflagged run can hard-fail the build (#836, spec D4a). The list mirrors
        // `rustc --print cfg --target wasm32-unknown-unknown`.
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        for feature in WASM_TARGET_FEATURES {
            assert!(
                args.contains(&format!("--enable-{feature}")),
                "missing --enable-{feature} in {args:?}"
            );
        }
    }

    #[test]
    fn wasm_opt_args_never_use_all_features() {
        // `-all` silently tracks whatever the installed binaryen considers "all",
        // so a binaryen upgrade could change the accepted input set with no diff.
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(!args.iter().any(|a| a == "-all" || a == "--all-features"), "{args:?}");
    }

    #[test]
    fn wasm_opt_args_name_input_and_output() {
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("a/in.wasm"), Path::new("b/out.wasm"));
        assert!(args.contains(&"a/in.wasm".to_string()), "{args:?}");
        let o = args.iter().position(|a| a == "-o").expect("has -o");
        assert_eq!(args[o + 1], "b/out.wasm", "{args:?}");
    }

    #[test]
    fn wasm_opt_does_not_request_debug_names() {
        // `-g` would retain the name section; the shipped bundle must not (spec D3).
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(!args.iter().any(|a| a == "-g"), "{args:?}");
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path tools/Cargo.toml csr_bundle` Expected: FAIL —
`wasm_opt_args` / `WASM_OPT_LEVEL` / `WASM_TARGET_FEATURES` not defined.

(`devtool` lives in the separate `tools/` virtual workspace,
`tools/Cargo.toml:3`, which the root workspace does not include — so
`-p devtool` from the root will not find it. This is the same invocation
`xtask/src/steps/host_tests.rs:22` uses.)

- [ ] **Step 3: Implement against the tests**

Add the constants and `wasm_opt_args` to signature in **Interfaces**. Every
element of the vector is pinned by a test — level, six enables, absence of
`-all`, absence of `-g`, input, `-o output`.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path tools/Cargo.toml csr_bundle` Expected: PASS.

- [ ] **Step 5: Run the pass in the pipeline**

In `run()` (`csr_bundle.rs:82`), after the rename and JS wasm-ref rewrite but
**before** `write_precompressed` (`:108-110`), run `wasm-opt` over
`out.join(OUT_WASM)` in place (write to a temp sibling, then rename over it).
Follow the existing `Command::new("wasm-bindgen")` error style (`:85-95`),
including the "is it on PATH?" context.

Precompression must run **after** wasm-opt so `.br`/`.gz` describe the optimised
bytes.

Update the module doc comment (`:1-12`) to say the step now optimises as well as
post-processes.

- [ ] **Step 6: Measure the three levels and pin the winner**

For each of `-O2`, `-Os`, `-Oz`: set `WASM_OPT_LEVEL`, run
`nix build .#site --no-link --print-out-paths`, then
`cargo xtask audit-wasm --site-path <path>`, and record raw bytes.

Pin the smallest. Record all three in the PR description (A9).

- [ ] **Step 7: Commit**

```bash
cargo xtask check
git add tools/devtool/src/csr_bundle.rs
git commit -m "feat(devtool): run wasm-opt over the CSR bundle (#836)"
```

---

## Task 9: Assert the shipped bundle carries no name section

A10 — verified by a test, not by inspection.

**Files:**

- Modify: `xtask/src/audit_wasm.rs` (+ its `mod tests`)

**Interfaces:**

- Consumes: `wasm_sections::section_sizes`.
- Produces: `pub fn has_name_section(wasm: &[u8]) -> anyhow::Result<bool>;`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn detects_a_present_name_section() {
        // The Task 4 fixture builder emits a name section.
        let wasm = crate::wasm_symbols::tests_support::named_module();
        assert!(has_name_section(&wasm).unwrap());
    }

    #[test]
    fn detects_an_absent_name_section() {
        let wasm = crate::wasm_symbols::tests_support::unnamed_module();
        assert!(!has_name_section(&wasm).unwrap());
    }
```

In Task 4's `wasm_symbols.rs`, promote the two fixture builders into a
`#[cfg(test)] pub mod tests_support { pub fn named_module() -> Vec<u8>; pub fn unnamed_module() -> Vec<u8>; }`
so both modules share them rather than duplicating.

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml name_section` Expected: FAIL —
`has_name_section` not defined.

- [ ] **Step 3: Implement against the tests**

`has_name_section` returns whether `section_sizes` yields a section named
`custom:name`. Both branches are pinned.

- [ ] **Step 4: Assert it on the shipped artifact**

In `audit_wasm::run` (`:146`), after reading each artifact, error if the
artifact is the wasm **and** `has_name_section` is true, with a message naming
#836 and D3. This makes every `audit-wasm` run — including the Task 15 budget
step in `validate` — a guard against silently losing the strip.

- [ ] **Step 5: Verify end-to-end**

Run: `cargo xtask audit-wasm` Expected: PASS, and raw bytes below the Task 6
baseline.

- [ ] **Step 6: Commit**

```bash
cargo xtask check
git add xtask/src/audit_wasm.rs xtask/src/wasm_symbols.rs
git commit -m "feat(xtask): guard that the shipped wasm carries no name section (#836)"
```

---

## Tasks 10–13: the `common` → `host` cluster moves

**Execute a task only if Task 6 marked its cluster material (≥ 25 KiB).** A
cluster below the threshold is filed in Task 19 instead. All four share the
shape below; each names its own files.

**Shared constraints (Global Constraints, D6):**

- No `#[cfg(feature)]` / `#[cfg(target_arch)]` added to `common/src` (A13).
- The cluster's feature flag and **every** one of its cfg sites are removed from
  `common` (A14).
- `host` already depends on `common` (`host/Cargo.toml:16`); `storage` already
  depends on `host` (`storage/Cargo.toml:13`). No new edges are needed.

**Shared step shape:**

- [ ] **Step 1:** Move the modules/functions listed in the task to `host/src/`,
      updating `host/src/lib.rs`.
- [ ] **Step 2:** Move the dependencies from `common/Cargo.toml` to
      `host/Cargo.toml` (they are not duplicated — `common` must lose them).
- [ ] **Step 3:** Update every call site named in the task.
- [ ] **Step 4:** Run `cargo nextest run -p common -p host -p storage -p server`
      — expected PASS, no behavior change.
- [ ] **Step 5:** Run `cargo check -p csr --target wasm32-unknown-unknown` —
      expected PASS. This is the only step that proves the wasm side still
      _compiles_; the suite above and `cargo tree` are both host-side, so
      without it A17 is unverified.
- [ ] **Step 6:** Run
      `cargo tree -p csr --target wasm32-unknown-unknown | rg '<dep>'` for each
      dep dropped — expected **no output** (A16).
- [ ] **Step 7:** Verify A13 — no cfg was added to escape the move:

```bash
rg -n '#\[cfg\((feature|target_arch)' common/src | wc -l
```

Expected: a count **no greater** than before the task (record it in the commit
body). For Tasks 11 and 13 it must _decrease_ by the cluster's sites (A14).

- [ ] **Step 8:** `cargo xtask check`, then commit.

### Task 10: syndication cluster _(if material)_

**Files:** move `common/src/atompub/**`, `common/src/feed/rss.rs`,
`common/src/feed/atom.rs` → `host/src/`. Modify `common/src/feed/mod.rs:19,22`
(drop the `render_rss`/`render_atom` re-exports), `common/Cargo.toml:25,26,27`
(drop `rss`, `atom_syndication`, `quick-xml`), `host/Cargo.toml`.

**Call sites to update:**
`server/src/atompub/{mod,posts,service,rsd,media,mapping}.rs`,
`server/src/lib.rs`, `server/src/feed/regenerate.rs`, `server/tests/*`.

**Stays in `common`:** `feed_path.rs`, `event_status`, `settings`, `window`,
`json` — `web` consumes
`FeedSurface`/`FeedFormat`/`canonicalize`/`affected_feed_urls` from them.

Commit: `refactor(common,host): move syndication rendering to host (#836)`

### Task 11: markup cluster _(if material)_

**Files:** move `mod sanitized` (`common/src/render.rs:240`) →
`host/src/render.rs`. Modify `common/src/render.rs:239,594,1133` (the three
`sanitize` cfg sites — **removed, not relocated**),
`common/Cargo.toml:8,15,17,19,52` (drop `ammonia`, `html5ever`, `orgize`,
`pulldown-cmark`, and the `sanitize` feature), `storage/Cargo.toml:12` (drop
`common/sanitize`), `host/Cargo.toml`.

**Stays in `common`:** `RenderedHtml`, `PostFormat`, `canonicalize_org_body`
(`render.rs:730` — pure string handling, no orgize).

**Extra step, between Steps 1 and 3:** add a minting door on `RenderedHtml` so
`host` can construct one from sanitized input (today `sanitize()` is in-module).
Keep it as narrow as the existing door — this is the type's whole safety
property (ADR trail via `rendered_html_from_trusted_check`, which `validate`
enforces).

Commit:
`refactor(common,host): move markup rendering and sanitization to host (#836)`

### Task 12: etag cluster _(if material)_

**Files:** move the `sha2`-based hashing constructor from `common/src/etag.rs`
and `common/src/feed/metadata.rs::feed_etag` → `host/src/etag.rs`. Modify
`common/Cargo.toml:29` (drop `sha2`), `host/Cargo.toml:13` (already has `sha2`).

**Stays in `common`:** the `ETag` type itself and its parsing, so
`common/src/test_support.rs:17` `parse_etag` keeps compiling for wasm (A17).

**Call sites to update:** `storage/src/posts.rs`, `storage/src/feed_cache.rs`,
`server/src/{site,media,feed/handlers,feed/regenerate,atompub/mod,atompub/posts}.rs`,
`server/src/projector/mod.rs`.

Commit: `refactor(common,host): move etag hashing to host (#836)`

### Task 13: kdf cluster _(if material)_

**Files:** move `Password::hash` (`common/src/password.rs:97`) and
`Password::verify` (`:133`) → free functions in `host/src/password.rs`, taking
`&Password` via its existing `AsRef<str>` (already used at `password.rs:232`).
Modify `common/Cargo.toml:9` (drop `argon2`), `common/Cargo.toml:53,54` (move
`cheap-kdf`/`test-utils` to `host`), `common/src/password.rs:109,116`,
`common/src/lib.rs:60` (remove the cfg sites), `host/Cargo.toml`.

**Stays in `common`:** `Password`, `ProfferedPassword`, and their `FromStr`
policy check — the client validates against them (ADR-0065, A17).

**Call sites to update:** `storage/src/helpers.rs:397,434,473`,
`storage/src/users.rs:263,353,361,455`, `storage/src/postgres/mod.rs:116,202`,
`storage/src/sqlite/mod.rs:232,326`.

**Watch:** the `test-utils`/`cheap-kdf` feature move must keep test-time KDF
cheapness working for `storage`'s tests, or the suite gets dramatically slower.
Verify by timing `cargo nextest run -p storage` before and after.

Commit: `refactor(common,host): move password hashing to host (#836)`

---

## Task 14: Manifest hygiene

Independent of the cluster verdicts (A19, A20).

**Files:**

- Modify: `web/Cargo.toml:32`, and — **only if Task 11 did not run** —
  `common/Cargo.toml:17,19`

- [ ] **Step 1: Remove the vestigial `croner` declaration**

`web/Cargo.toml:32` declares `croner.workspace = true` with no live `croner::`
call site in `web/src` (only a comment at `web/src/forms/field.rs:190`). Remove
the line.

- [ ] **Step 2: Verify nothing broke**

Run: `cargo check -p web --no-default-features --features csr` Expected: PASS.

Run: `cargo tree -p csr --target wasm32-unknown-unknown | rg croner` Expected:
still present — `croner` reaches wasm through `common`'s `BackupSchedule`, which
is deliberate (D7, A18). This step confirms the removal was a manifest cleanup,
not a behavior change.

- [ ] **Step 3: Fix the `orgize`/`pulldown-cmark` non-optionality — only if Task
      11 did not run**

If the markup cluster moved, they are already gone; skip. Otherwise mark both
`optional = true` in `common/Cargo.toml:17,19` and add them to the `sanitize`
feature list (`:52`), since their only users are already gated behind it. Then:

Run: `cargo tree -p csr --target wasm32-unknown-unknown | rg 'orgize|pulldown'`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
cargo xtask check
git add web/Cargo.toml common/Cargo.toml Cargo.lock
git commit -m "chore(web,common): drop vestigial and never-used wasm deps (#836)"
```

---

## Task 15: The raw-byte budget in `validate`

**Files:**

- Create: `xtask/src/wasm_budget.rs`, `xtask/src/steps/wasm_budget.rs`
- Modify: `xtask/src/lib.rs` — `mod wasm_budget;` in the module list at
  `:24-51`, `pub mod wasm_budget;` inside the **inline** `mod steps { … }` block
  (there is no `xtask/src/steps/mod.rs`), the `:127` doc comment, and the
  `validate` wiring at `:478-501`
- Modify: root `Cargo.toml:16-20` (A30, its own commit in Step 9)

**Interfaces:**

- Consumes: `audit_wasm::run` (the shipped-artifact measurement).
- Produces:

  ```rust
  /// Raw bytes of `pkg/jaunder.wasm`, the volume firefox must compile.
  ///
  /// Achieved: <N> bytes (#836). Headroom: <H> bytes (~<P>%).
  /// RAW, not gzip/brotli: compression governs transfer, but the wasm
  /// compiler's input is the decompressed artifact, and compile time is what
  /// dominates the boot gap (#818). Lower this deliberately when a win lands.
  pub const WASM_RAW_CEILING_BYTES: u64 = <N + H>;
  /// The size actually achieved by #836, recorded so the headroom is a checkable
  /// fact rather than a claim in a comment.
  pub const WASM_RAW_ACHIEVED_BYTES: u64 = <N>;
  pub struct BudgetVerdict { pub actual: u64, pub ceiling: u64, pub over: bool }
  pub fn check(actual: u64, ceiling: u64) -> BudgetVerdict;
  pub fn failure_message(v: &BudgetVerdict) -> String;
  ```

- [ ] **Step 1: Write the failing tests**

In `xtask/src/wasm_budget.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_ceiling_passes() {
        let v = check(1_000, 2_000);
        assert!(!v.over);
    }

    #[test]
    fn exactly_at_ceiling_passes() {
        // The ceiling is inclusive: a build that exactly hits it is not a regression.
        let v = check(2_000, 2_000);
        assert!(!v.over);
    }

    #[test]
    fn over_ceiling_fails() {
        let v = check(2_001, 2_000);
        assert!(v.over);
    }

    #[test]
    fn failure_message_states_actual_ceiling_and_the_remedy() {
        let m = failure_message(&check(3_000, 2_000));
        assert!(m.contains("3000") || m.contains("3,000"), "{m}");
        assert!(m.contains("2000") || m.contains("2,000"), "{m}");
        assert!(
            m.contains("WASM_RAW_CEILING_BYTES"),
            "must name the constant to change: {m}"
        );
        assert!(m.to_lowercase().contains("raw"), "{m}");
    }

    #[test]
    fn the_committed_ceiling_has_headroom_over_the_achieved_size() {
        // Guards the D8 decision itself: a ceiling equal to the achieved size is a
        // strict ratchet, which the spec rejects.
        assert!(
            WASM_RAW_CEILING_BYTES > WASM_RAW_ACHIEVED_BYTES,
            "ceiling {WASM_RAW_CEILING_BYTES} must exceed achieved {WASM_RAW_ACHIEVED_BYTES}"
        );
    }

    #[test]
    fn the_achieved_size_is_below_the_issue_836_baseline() {
        // The point of the cycle: 5 350 591 was the pre-cut raw size.
        assert!(WASM_RAW_ACHIEVED_BYTES < 5_350_591, "{WASM_RAW_ACHIEVED_BYTES}");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_budget` Expected: FAIL —
module not defined.

- [ ] **Step 3: Implement against the tests**

Signatures as in **Interfaces**. Every branch is pinned: under, exactly-at,
over, and the message's four required elements. Set `WASM_RAW_CEILING_BYTES`
from the post-Task-14 measured size plus explicit headroom; state both numbers
in the doc comment.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml wasm_budget` Expected: PASS (6
tests).

- [ ] **Step 5: Add the `validate` step**

`xtask/src/steps/wasm_budget.rs`:

```rust
//! `validate`'s wasm size budget (#836). Reads the same measurement
//! `audit-wasm` produces for the shipped artifact, so the gate and the tool can
//! never disagree.
pub fn run(result: &mut crate::result::CommandResult) {
    match crate::audit_wasm::run(None) {
        Ok(report) => {
            let wasm = report
                .artifacts
                .iter()
                .find(|a| a.path.ends_with(".wasm"))
                .map(|a| a.raw_bytes);
            // Keep the measurement: `validate --json` already paid for the
            // `nix build .#site`, so discarding the sizes would waste it.
            result.audit = Some(report);
            match wasm {
                Some(raw_bytes) => {
                    let v = crate::wasm_budget::check(
                        raw_bytes,
                        crate::wasm_budget::WASM_RAW_CEILING_BYTES,
                    );
                    if v.over {
                        result.push(
                            crate::result::StepResult::fail("wasm-budget")
                                .detail(crate::wasm_budget::failure_message(&v)),
                        );
                    } else {
                        result.push(
                            crate::result::StepResult::ok("wasm-budget")
                                .detail(format!("{} raw (ceiling {})", v.actual, v.ceiling)),
                        );
                    }
                }
                None => result.push(
                    crate::result::StepResult::fail("wasm-budget")
                        .detail("audit-wasm reported no .wasm artifact"),
                ),
            }
        }
        Err(e) => result.push(
            crate::result::StepResult::fail("wasm-budget").detail(format!("{e:#}")),
        ),
    }
}
```

Declare `pub mod wasm_budget;` inside the **inline** `mod steps { … }` block at
`xtask/src/lib.rs:24-51` — there is no `xtask/src/steps/mod.rs`. Wire the call
into `validate` (`:478-501`), after `steps::e2e_scaffold_check::run` and before
`steps::host_tests::run`.

- [ ] **Step 6: Correct the stale doc comment**

`xtask/src/lib.rs:127` currently reads _"This is a manual tool — it is not part
of `check`/`validate`."_ Replace with a line stating that the totals now back
`validate`'s `wasm-budget` step, while `--breakdown` remains manual (A24).

- [ ] **Step 7: Verify**

Run: `cargo xtask validate --no-e2e` Expected: PASS, with a `wasm-budget` step
reporting actual vs ceiling.

Temporarily lower `WASM_RAW_CEILING_BYTES` to 1, re-run, and confirm the failure
message names the constant. Restore it.

- [ ] **Step 8: Commit**

```bash
cargo xtask check
git add xtask/src/wasm_budget.rs xtask/src/steps/wasm_budget.rs xtask/src/lib.rs
git commit -m "feat(xtask): gate raw wasm size against a committed ceiling (#836)"
```

- [ ] **Step 9: Record the `panic = "abort"` rejection (A30) — its own commit**

Unrelated to the budget, so it does not ride that commit. Add to the root
`Cargo.toml` `[profile.release]` block (`Cargo.toml:16-20`):

```toml
# Deliberately no `panic = "abort"`. It is already the default for
# wasm32-unknown-unknown (`rustc --print cfg`), so it buys zero wasm bytes —
# and here it would apply workspace-wide, flipping the server's release build
# to abort so a panicking tokio task kills the process instead of being
# isolated (#836).
```

```bash
git add Cargo.toml
git commit -m "docs(cargo): record why panic=abort is not set (#836)"
```

---

## Task 16: ADR draft for the budget

D8/D10, A29. Uses `jaunder-adr`'s draft-out-of-git flow — numberless in
`docs/adr/drafts/`, numbered at ship by `cargo xtask adr promote`.

**Files:**

- Create: `docs/adr/0102-wasm-raw-size-budget.md`

- [ ] **Step 1: Write the draft**

Follow `jaunder-adr` for the required structure. Content:

- **Context:** #818 attributed 80.5–87.6% of the firefox/chromium boot gap to
  wasm compile+instantiate, at ~88 ms per MiB of raw wasm. #836 cut the bundle;
  nothing stopped it growing back.
- **Decision:** `cargo xtask validate` fails when raw `pkg/jaunder.wasm` exceeds
  `WASM_RAW_CEILING_BYTES`, a committed constant carrying explicit headroom
  above the achieved size.
- **Why raw and not brotli** — the load-bearing part. Compression governs
  transfer; the wasm compiler's input is the decompressed artifact. A budget on
  the compressed figure would be satisfied by a change that compresses better
  while compiling slower. State that reviewers will be tempted to "fix" this and
  that it must not be fixed.
- **Why headroom, not a strict ratchet:** a zero-headroom ceiling turns red on
  any innocent dependency bump; the fix is always "raise the number", which
  erodes the gate's authority. Headroom is explicit and reviewable instead of
  implied.
- **How it is lowered:** deliberately, in the same commit as the win that earned
  it.
- **Consequences:** `validate` gains a `nix build .#site`; the ceiling is a real
  review surface.
- **Relationship to ADR-0028:** unchanged — that ADR governs devtool-vs-xtask
  placement, and the budget lives in xtask for exactly its stated reason
  (host-side analysis).

- [ ] **Step 2: Verify the draft passes the ADR check**

Run: `cargo xtask check` Expected: PASS — `adr_check` accepts the draft.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0102-wasm-raw-size-budget.md
git commit -m "docs(adr): draft the raw wasm size budget (#836)"
```

---

## Task 17: Commit the pre-registered prediction

**This must be committed before Task 18 runs** — A26 is checked by git ancestry,
not by assertion.

**Files:**

- Modify: `docs/observability.md` (a **"Prediction (pre-registered)"**
  subsection under the #836 section created in Task 6)

`docs/superpowers/` holds only `plans/` and `specs/`, and `jaunder-ship`
archives by those names — a new `predictions/` directory would be an invented
convention whose file might not survive ship. The prediction lives in
`docs/observability.md` instead, where Task 18's result will sit directly
beneath it.

- [ ] **Step 1: Compute the prediction**

From Task 6's baseline raw bytes, the post-cut raw bytes
(`cargo xtask audit-wasm`), and 88 ms/MiB:

```
predicted_drop_ms = (baseline_bytes - achieved_bytes) / 1_048_576 * 88
```

- [ ] **Step 2: Write it down**

The subsection states: baseline bytes, achieved bytes, the delta, the 88 ms/MiB
constant and its provenance (#818), the arithmetic, the predicted firefox
compile-time drop, and Task 7's measured baseline compile figure so the
predicted _after_ figure is explicit.

State plainly that this is written before the after-capture is run, and why:
#840 had to withdraw a claim inferred from timing shape rather than tested.

- [ ] **Step 3: Commit — before running Task 18**

```bash
git add docs/observability.md
git commit -m "docs(observability): pre-register the predicted compile-time drop (#836)"
```

- [ ] **Step 4: Record the commit sha**

A26 is checked by ancestry — note this commit's sha in the PR description so a
reviewer can confirm it precedes Task 18's without reconstructing the history.

---

## Task 18: After-capture and the write-up

**Files:**

- Create: `~/measurements/jaunder/issue-836-wasm-shrink/after/` (outside the
  repo)
- Modify: `docs/observability.md` (including the dangling `:642` line)

- [ ] **Step 1: Run the after-capture**

Repeat Task 7's `protocol.md` exactly, on the current HEAD, into `.../after/`.

- [ ] **Step 2: Summarize on the same shape**

Median wasm fetch / compile / instantiate per browser and population, as in the
before summary.

- [ ] **Step 3: Write it up**

Extend `docs/observability.md`'s #836 section with: before and after raw bytes
from `cargo xtask audit-wasm` (A25); the wasm-opt level comparison; **predicted
vs observed** firefox compile time, stating whether the 88 ms/MiB relationship
held (A27); and the capture protocol, capture counts and summary statistics, so
a reader can review the claim without the corpus.

If observed diverges materially from predicted, **say so and leave it
unexplained** rather than supplying a story — that is the #840 lesson this cycle
is built on.

- [ ] **Step 4: Fix the dangling line**

`docs/observability.md:642` says _"Levers and the unverified streaming question
are #836."_ Update it: the streaming question was answered by #840 (withdrawn,
unexplained), and the levers were exercised here — link the #836 section (A28).

- [ ] **Step 5: Commit**

```bash
git add docs/observability.md
git commit -m "docs(observability): before/after wasm size and boot capture (#836)"
```

---

## Task 19: File the remaining conditional issues

A12, A31, A32. Driven by Task 6's verdicts.

- [ ] **Step 1: File one issue per sub-threshold cluster**

For each D6 cluster Task 6 marked below 25 KiB, file an issue (via
`jaunder-issues`) recording its measured bytes and stating that the ADR-0058
layering argument for moving it stands independently of bundle size — it simply
is not #836's business. Reference the Task 1 `common`-split issue.

- [ ] **Step 2: File the croner issue if warranted**

If `croner` measured ≥ 25 KiB (D7): file an issue for a smaller cron validator
or a syntactic/full validation split, quoting ADR-0065's requirement that client
and server validate through the _same_ function, and recording the measured
bytes.

- [ ] **Step 3: File the log-string issue if warranted**

If `log` + `console_log` measured ≥ 25 KiB (D9): file an issue for
`release_max_level_*`, noting it must be sequenced **against #839**, which
exists to start capturing browser console output.

- [ ] **Step 4: Cross-reference**

Add every filed issue to the spec's _Deferred_ list, and collect them for the PR
description.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-08-05-issue-836-shrink-wasm-bundle.md
git commit -m "docs(spec): link the deferred issues filed from the measurement (#836)"
```

---

## Self-review notes

**Spec coverage.** A1→T3/T5; A2→T4/T5; A3→T5; A4→T5; A5→T3/T4 (with the
`wasm-encoder`-synthesized-fixture deviation stated in T3); A6→T6; A7→T2/T8;
A8→T8; A9→T8; A10→T9; A11→T8 (one `wasm_opt_args`, used by the single `run()`
both paths call); A12→T6/T10–13/T19; A13→T10–13 Step 7; A14→T11/T13 Step 7;
A15→T11; A16→T10–13 Step 6; A17→T10–13 Step 5
(`cargo check -p csr --target wasm32-unknown-unknown`) + T12/T13; A18→T14;
A19→T14; A20→T11/T14; A21→T15; A22→T15 (`WASM_RAW_ACHIEVED_BYTES` makes the
headroom checkable); A23→T15; A24→T15; A25→T18; A26→T17 (ancestry, plus the sha
recorded in the PR); A27→T18; A28→T18; A29→T16; A30→T15 Step 9 (its own commit);
A31→T6/T19; A32→T1/T19.

**Placeholder scan.** No TBD/TODO; each implementation step names a signature
and the tests pinning it. Tasks 10–13 are conditional by design (D0) with the
condition stated and its decision procedure in Task 6, not left to taste.

**Type consistency.** `SectionSize`/`section_sizes`/`assert_spans_cover` (T3)
used by T5/T9; `FunctionSize`/`CrateBytes`/`rollup`/`UNATTRIBUTED` (T4) used by
T5; `tests_support::{named_module, unnamed_module}` (T4) used by T9;
`BreakdownReport`/`breakdown`/`render_breakdown` (T5) used by the CLI;
`wasm_opt_args`/`WASM_OPT_LEVEL`/`WASM_TARGET_FEATURES` (T8) internal to
`csr_bundle`;
`WASM_RAW_CEILING_BYTES`/`WASM_RAW_ACHIEVED_BYTES`/`check`/`failure_message`/`BudgetVerdict`
(T15) used by the step. Names match across tasks.

**Verified against the repo** (plan soundness review): `xtask` tests run via
`cargo test --manifest-path xtask/Cargo.toml` and `devtool` via
`cargo test --manifest-path tools/Cargo.toml` — both as
`xtask/src/steps/host_tests.rs:16,22` invokes them. `steps` is an **inline**
module in `xtask/src/lib.rs:24-51`; there is no `steps/mod.rs`.
`CommandResult::push` (`result.rs:95`), `StepResult::ok/fail/detail`
(`:22,30,46`), the `audit` field (`:62`), and `audit_wasm::run(Option<&str>)`
(`audit_wasm.rs:146`) are all real. `cargo xtask check` is the pre-commit gate
(`CONTRIBUTING.md:82`) and its `adr_check` gates only numbered
`docs/adr/NNNN-*.md`, so a numberless draft passes.
