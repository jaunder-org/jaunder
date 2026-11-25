# Issue #2101 — move `wasm-audit` into `dev` as Python, with twiggy

## Outcome

`dev wasm-audit report` and the `wasm-budget` step in `validate` run as Python
inside `tools/dev`. The Rust `wasm-audit` executable and its wasm parsing
modules are deleted. `report --breakdown` uses `twiggy`, a maintained tool now
in the devShell, to attribute bytes to crates.

This implements ADR-0261's #1974 verdict.

## Approach and tradeoffs

The new module is `tools/dev/dev/wasm_audit.py`. It has three parts:

1. **Bundle measurement.** `report` and the budget share it.
   - It reads `<root>/.vite/manifest.json` and takes the wasm from the `file` of
     the `csr_bg.wasm` entry, which must end in `.wasm`.
   - Every entry must have a `file`. Each `css` and `assets` field must be an
     array of strings. Every listed path must be relative and safe, and must
     exist under the root. These checks carry over from `host::vite_manifest`.
   - The `index.html` entry is not required. Python never needs the entry
     JavaScript on its own, and `server/build.rs` still validates it.
   - The selected outputs are the wasm plus every `.js` file the manifest lists.
     For each one, Python reports the raw size and the sizes of the `.gz` and
     `.br` files Vite writes next to it. Python compresses nothing.
   - The key `csr_bg.wasm` is now spelled in both Rust and Python. A new row in
     ADR-0109's literal table (`xlang_literal_check.rs`) pins the two spellings.
     Its Rust anchor is `let wasm = file_for(` in `host/src/vite_manifest.rs`;
     its Python anchor is the module's constant.
2. **Section walk**, used by `--breakdown` only. It reads each section's id byte
   and its LEB128 length, and for a custom section, its name. It interprets no
   section contents (ADR-0261). The header plus the section spans must equal the
   file length, or the walk fails, as the Rust walk does.
3. **Crate rollup**, used by `--breakdown`.
   - It runs `twiggy top -f json` with twiggy's own default of all items.
   - It drops twiggy's items that are not functions:
     - `"…" subsection`, `data segment …`, `custom section …`;
     - `export …` and `import …`;
     - `type[N]`, `elem[N]`, `global[N]`, `table[N]`, `memory[N]`;
     - `wasm magic bytes` and `… section headers`.
   - Each remaining item goes to a crate under the existing rule:
     - the first path segment, with any `[hash]` removed;
     - for `<Self as Trait>::m`, the crate of `Self`, or of `Trait` when `Self`
       is a primitive;
     - otherwise `<unattributed>`. This covers `code[N]` and unmangled
       wasm-bindgen shims.
   - Rows sort by bytes descending, then by name.

**Denominators (decided by the human).**

- Section shares are shares of the file.
- Crate shares are shares of twiggy's function total, labelled "code + function
  sections". twiggy charges each function for its body plus its function-section
  entry, so against the code section alone the rows would sum to about 100.8%.
  With this denominator they sum to 100%.

**Budget.** `gates.wasm_budget()` builds `.#csrBundle` as its own step. Inside a
`wasm-budget` phase, it then takes the raw size of the selected wasm, so the
budget never runs Nix.

- The ceiling is still inclusive: only `actual > ceiling` fails.
- Two constants move into `wasm_audit.py`: `WASM_RAW_CEILING_BYTES = 5_000_000`
  and `WASM_RAW_ACHIEVED_BYTES = 4_009_656`.
- A pass prints the size, the ceiling and the drift from #1549 to stdout.
- A failure prints to stderr and raises `steps.StepFailed`, as `perf._refuse`
  does. The message gives both numbers, says the budget is on raw bytes, names
  the constant and its file, asks for a deliberate raise with a reason in the
  commit, and points to `--breakdown`.
- `twiggy` runs through `steps.output`, and the Nix builds through
  `steps.nix_build`. Every subprocess is therefore a traced step.

**The strip check is dropped.** Rust's `report` and budget fail if the shipped
wasm carries a `name` section. The ceiling already covers that: the headroom is
990 KB, and the name section was 1.28 MiB in #836. That check was the only thing
that parsed the shipped file. ADR-0106's amendment records the change.

**CLI (decided by the human).**

- `dev wasm-audit report [--site-path P] [--breakdown [--wasm F]]`.
  - `--wasm` requires `--breakdown`.
  - `--site-path` together with `--breakdown` is a usage error. Rust silently
    ignores it.
  - With no path, `report` builds `.#csrBundle`, and `--breakdown` builds
    `.#csrWasm`.
- The `budget` subcommand is dropped. Only the gate calls the budget, and
  `report` already shows the raw size.
- The JSON report is dropped, because nothing reads it. `dev` prints the tables
  itself.

**Deleted.**

- `tools/host-proof` files: `bin/wasm-audit.rs`, `audit_wasm.rs`,
  `wasm_budget.rs`, `steps/wasm_budget.rs`, `wasm_sections.rs`,
  `wasm_symbols.rs`.
- Their only dependencies: `wasmparser`, `wasm-encoder`, `rustc-demangle`.
- The `wasm-audit` entry in `hostProofBin.bins`, and its `dev` forwarding entry.
- The `dev` tests that use `wasm-audit` as a sample host-proof report tool
  (`test_report.py`, `test_workflows.py`, `conftest.py`, `test_gates.py`). They
  switch to `trace-analysis`, or move to the new module's tests.

**Rejected alternative.** Keeping the Rust parsing and moving only `budget`
would leave parsers that duplicate twiggy (ADR-0261).

**Accepted cost.**

- Crate byte counts are within about 1–5% of the Rust rollup, with the same
  ranking. The probe gave reactive_graph 1,438,016 B against the Rust table's
  1.4 MiB, for example.
- Attribution now depends on twiggy's item names. nixpkgs pins twiggy at 0.8.0.

## Acceptance

**Proof to run.**

- `dev-python` stays green at 100% branch coverage. New pytest tests cover:
  - manifest selection and each rejection;
  - the section walk on hand-built bytes, a truncated file and a non-wasm file;
  - budget boundaries: at the ceiling, one byte over, and the text of the
    remedy;
  - the ADR-0106 constant tests carried over from Rust:
    - the achieved size passes with headroom;
    - the historical `-Os` and `-O2` sizes fit;
    - the pre-#836 size fails;
  - the rollup rules and every dropped item kind, from twiggy JSON fixtures;
  - the CLI usage errors;
  - one real `twiggy` run on a small module with a name section. To allow it,
    twiggy is added to `dev-python`'s inputs.
- The literal-pair gate passes, and a mismatched spelling fails it.
- `dev wasm-audit report`, `report --breakdown` and the wasm-budget step of
  `validate-no-e2e` run on real artifacts. The PR shows their output. It also
  compares the crate table with the Rust `--breakdown` from main on the same
  `.#csrWasm`, and names each divergence.

**To commit.**

- The Python module and its tests.
- twiggy in the devShell's `ciInputs` (host pytest runs it in CI's shell) and in
  `dev-python`.
- The Rust deletions, with an updated `Cargo.lock`.
- The pair-table row.
- An updated comment on `csrWasm` in `nix/packages.nix` naming the new consumer.
- ADR amendments:
  - ADR-0106: the constant's file, twiggy attribution, the denominator and the
    dropped strip check;
  - ADR-0246 and ADR-0247 (the tool leaves the JSON-report list);
  - ADR-0251: the budget runs in-process;
  - ADR-0258.
- `CONTRIBUTING.md`, `docs/observability.md` and `docs/ARCHITECTURE.md`,
  including the stale claim that `.#site` is retained only for `wasm-audit`
  (`docs/ARCHITECTURE.md:3123`).

## Boundaries

- The ceiling, the raw-byte semantics, the `wasm-opt` level and the Nix
  artifacts do not change.
- No other host-proof tool moves.
- No twiggy-based gate is added; `--breakdown` stays manual.
