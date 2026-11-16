# Issue #1958: split the `_host` recipe into named check groups

## Outcome

The host check surface (`_host`) becomes a short list of five public, named
check groups. A developer can run any one group, or one source contract by name,
and a failure names the kind of check that failed rather than "`_host`".

## Approach and tradeoffs

`_host mode docs` keeps its signature and callers (`check-no-test`, `prepush`,
`validate-no-e2e`, `ci-validate-host`). Its 44-line body is replaced by a
dependency list of the groups, which Just runs in order and stops at the first
failure:

| Group (public recipe)              | Leaves, in order                                                                                                      |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `format mode="verify" docs="host"` | `fmt`, `leptosfmt`, the Markdown slot and Prettier `end2end` (`_markdown`), Prettier `css`, `elisp-fmt`, `tools-fmt`  |
| `docs-shape`                       | the current `_docs-shape` leaves, which it renames (sequence, ADR format and parity, links, flow docs)                |
| `source-contracts`                 | `ast-grep-tests`, `no-full-reload`, the 22 `repository-policy source-contracts` policies, then `doctest-check fences` |
| `compile-checks mode="verify"`     | `tsc`, `cargo-deny`, the six Clippy units                                                                             |
| `elisp-checks mode="verify"`       | `byte-compile`, ERT (`devtool check ert`, with `--fix` in Fix mode)                                                   |

- **Markdown slot.** The existing private `_markdown mode` (Prettier Markdown,
  then `end2end`) gains `docs="host"` and becomes the one recipe that holds the
  prepush swap. With `docs=prepush` on x86_64-linux its first line runs
  `git-index-safety prepush-docs`; otherwise it runs Prettier Markdown in
  `mode`. `format` calls it as a body line, `just _markdown {{mode}} {{docs}}`,
  the same re-entry pattern `_host` already uses for `leptosfmt` and
  `_docs-shape`. This replaces `_host`'s inline string-concatenated conditional
  without adding a recipe, and `precommit-markdown-host` keeps calling
  `(_markdown "fix")` unchanged. `docs` accepts only `host` or `prepush`, and
  `prepush` requires `verify`; anything else fails through `error()`, as
  `recipes/static.just` does for `mode`.
- **`source-contract name`** is a new public one-liner that runs one policy
  (`repository-policy source-contracts -- <name>`). The group lists its policies
  as body lines with the same command rather than calling `just source-contract`
  22 times, which would re-enter Just for each one.
- **Order change (approved).** Leaves are regrouped by kind, so three moves
  happen: the ast-grep checks move from before docs shape to the start of source
  contracts, `doctest-check fences` moves from the middle of the policies to the
  end, and `byte-compile` moves from before `tsc` to the Elisp group. Every move
  comes after the formatters, so Fix-mode rewrites still land before any
  verdict. The only effect is which failure is reported first.
- **Groups are public**, so `just --list` shows them. A private recipe can still
  be run by name, but nobody finds a hidden one. Each group and
  `source-contract` gets a one-line `#` doc comment for the listing.

## Acceptance

- `tools/host-proof/tests/just_workflows.rs` replaces `broad()` with one helper
  per group, concatenated in `_host`'s order. This moves `ast-grep-tests` and
  `no-full-reload` out of the formatter list, `doctest-fences` to the end of the
  policies, and `byte-compile` to the Elisp helper. The narrow Markdown test
  (`markdown_host_runs_the_complete_narrow_fix_surface_in_order`) reuses the
  `docs-shape` helper instead of repeating its leaves. The `check`,
  `check-no-test`, `precommit-broad-host`, `prepush`, `validate`,
  `validate-no-e2e` and `ci-validate-*` assertions still compare the complete
  leaf sequence with `assert_eq!`, covering the same leaves and population.
- New fake-tool tests run each public group, in both modes where it takes one,
  plus `source-contract <name>`, and assert that exactly that group's leaves
  run. Prepush's `prepush-docs` slot keeps its current assertion.
- The existing every-leaf fault-injection test still passes, so each leaf's
  failure still stops later work.
- An invalid `docs` value, or `docs=prepush` combined with `fix`, fails at the
  Markdown slot before any Markdown leaf runs. Just evaluates each body line
  just before running it, so the earlier formatters have already run.
- No recipe body in the gate graph exceeds 30 lines (`source-contracts`, the
  largest, has 25).
- CONTRIBUTING's `check-no-test` row and the surrounding text name the five
  groups and `source-contract`. The `prepush` row and
  `tools/repository-policy/README.md` still describe the Markdown slot
  accurately.
- Proof:
  `cargo test --manifest-path tools/Cargo.toml -p host-proof --test just_workflows`,
  then the commit and push hooks.

## Boundaries

- No change to any leaf command, `recipes/static.just`, `recipes/prettier.just`,
  the Nix checks or the CI workflow.
- No change to `precommit`'s Git/index ownership. `precommit-markdown-host` only
  follows the `_docs-shape` → `docs-shape` rename.
- The leaves stay as they are; the binaries are not given their own `all` mode.
