# Issue #2120 — retire the unused `packages.site` output

## Outcome

The flake no longer exports `packages.<system>.site`. `packages.jaunder` stays
the only deployable output, and the CSR bundle stays reachable as `.#csrBundle`
(and the unstripped module as `.#csrWasm`).

## Approach and tradeoffs

`site` (`nix/packages.nix`) copies `csrWasmBundle` minus its private
`system-artifacts` directory. A search of Nix, CI workflows, `dev`, Rust tools
and scripts finds two readers, both incidental:

- the flake's `packages` attribute set (`site = site;`);
- `nix/platform-contract.nix`, the evaluation-only regression that asserts each
  listed portable package exists on every supported system and forces its
  derivation on the native one.

Nothing builds or serves it. `dev wasm-audit` reads `.#csrBundle`/`.#csrWasm`,
the server embeds the bundle, and the NixOS module runs only the binary.

Changes:

- Delete the `site` derivation and its export from `nix/packages.nix`, and fix
  the two comments that name it (the `mkCsrWasm` header and the
  `diagnosticCsrWasmBundle` note).
- Drop `"site"` from `portable` in `nix/platform-contract.nix`.
- ADR-0142: add a second `- Amended:` line and an inline `_Amended by #2120:_`
  note recording that `packages.site` is removed, and adjust the Context and
  Consequences sentences that describe it as present. The "Serving
  `packages.site` in production" rejected alternative stays, reworded to a
  separate served site tree, since the reason still holds.
- `docs/ARCHITECTURE.md`: drop `packages.site` from the "What the flake ships"
  list and its following paragraph; reword the `.#site`/CSR closure and the
  "unrelated `.#site` realization" mentions so they name only existing outputs.
- `CONTRIBUTING.md`: drop the "(not the incomplete `.#site`)" aside.

Alternative: keep the output as a convenience. Rejected: it has no reader, every
audit and deployment path uses another output, and ADR-0142 already says it must
not be documented as a runtime bundle. Cost accepted: anyone running
`nix build .#site` by hand gets an evaluation error and must use `.#csrBundle`.

Historical ADR text (ADR-0028's script table, ADR-0106's Consequences) that
records `nix build .#site` as part of the tooling of its time stays unchanged;
it describes retired tools (`cargo xtask`, `scripts/audit-wasm-bundle`), not
current outputs.

## Acceptance

Proof to execute:

- `nix eval .#packages.x86_64-linux --apply builtins.attrNames` no longer lists
  `site`, and still lists `jaunder`, `csrBundle` and `csrWasm`.
- The platform-contract evaluation (the `platform` step of `dev validate-no-e2e`
  and `dev ci-validate-host`) passes when run directly.
- After this spec is retired, `rg '\.#site|packages\.site'` finds only ADR-0142
  (its amended text) and the historical ADR mentions above.
- The hook-backed `dev precommit`/`dev prepush` gates and CI pass.

Deliverables to commit: the Nix and doc changes above.

## Boundaries

- No change to `csrWasmBundle`, `csrBundle`, `csrWasm`, the server's embedding
  or the NixOS module.
- No rename of the internal `siteSrc`/`jaunder-site-cargo-source` filter; it is
  the CSR source closure, not the removed output.
