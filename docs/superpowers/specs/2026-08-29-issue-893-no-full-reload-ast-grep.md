# #893 — move no-full-reload policy to ast-grep

Issue: [#893](https://github.com/jaunder-org/jaunder/issues/893). Milestone:
Developer tooling & DX.

## Outcome

The `no-full-reload` static check recognizes forbidden raw browser navigation
from Rust syntax rather than individual source lines. Formatting a forbidden
call chain across lines cannot evade the gate, and the same rule runs in the
host verify ladder and the hermetic Nix static-check lane.

## Load-bearing decisions

- Preserve ADR-0076's policy exactly: Rust in `web/src` and `client/src` may not
  call `replace`, `assign`, `reload`, or `set_href` on the result of any
  `.location()` method call. Location inspection and `use_location()` remain
  allowed, and the pre-paint JavaScript redirect remains outside the Rust rule.
- Express the four forbidden AST shapes in a committed ast-grep YAML rule under
  a root `ast-grep/` policy directory, discovered through root `sgconfig.yml`.
  The rule is the single matching-policy definition.
- Make `no-full-reload` a `devtool check` definition. Host xtask lanes and the
  Nix `static-checks` derivation reach the same definition, following ADR-0052
  and ADR-0146; no independent xtask source scanner remains.
- Preserve the `no-full-reload` check identity and actionable diagnostic that
  directs callers to `leptos_router`'s `use_navigate()`.
- Supply ast-grep from the flake-locked nixpkgs package (`pkgs.ast-grep`; 0.45.1
  at design time). It belongs in the shared development/CI inputs and in the
  hermetic `static-checks` native inputs. No separate upstream source override
  or second version pin is introduced.
- The rule directory is part of `staticCheckSrc`, whose exclusion-only filter
  already admits it. Product and coverage source filters do not admit the rule:
  those derivations do not execute it, and coupling their hashes would be
  unrelated churn.
- Rule behavior is tested as rule behavior, including a call chain split across
  lines. Scanner implementation tests disappear with the scanner.
- Replace the prior no-ADR decision with the proposed
  [devtool ast-grep enforcement ADR](../../adr/drafts/devtool-owns-ast-grep-enforcement.md):
  it amends ADR-0076's enforcement mechanism while preserving its navigation
  policy, and records why devtool owns the native rule test and repository scan
  across host and Nix.
- Update `docs/ARCHITECTURE.md` to project the gate's devtool/ast-grep
  ownership, rule self-tests, repository scan, and host/Nix execution surfaces.

## Acceptance

- A forbidden call chain split before or after `.location()` fails
  `no-full-reload`; the same four call shapes on one line also fail.
- Allowed Rust examples—including `use_location()`, unrelated `replace` calls,
  and location reads without a forbidden navigation method—pass the rule.
- A clean repository passes `devtool check no-full-reload` and the host
  `cargo xtask check --no-test` surface that delegates to it.
- The hermetic Nix `static-checks` derivation contains ast-grep and runs the
  same `devtool` check definition through `devtool check --all --sandbox-cargo`.
- The old per-line matcher, its xtask registration, and its implementation-level
  tests are absent; there is no compatibility alias or duplicate policy path.
- The gate reports the violating file and source location with the existing
  router-navigation recovery guidance.
- `docs/ARCHITECTURE.md` no longer describes `no-full-reload` as an xtask source
  scan and accurately projects its devtool/ast-grep ownership.

## Boundaries

- Do not broaden the prohibition to every `.location()` use or to JavaScript,
  HTML, or non-navigation browser behavior.
- Do not migrate marker-bearing, allowlisted, counted, or budgeted xtask gates
  to ast-grep as part of this issue.
- Do not redesign the static-check execution ladder, flake source filters, or
  devtool process supervision beyond the wiring required for this check.
- Do not add ast-grep rewriting, editor integration, or unrelated rules.
