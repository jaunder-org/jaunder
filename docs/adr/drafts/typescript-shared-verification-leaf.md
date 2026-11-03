# ADR-DRAFT: Isolate the shared TypeScript check in a Nix leaf

- Status: proposed
- Date: 2026-11-03
- Issue: [#1864](https://github.com/jaunder-org/jaunder/issues/1864)

## Context

The host static ladder and Nix `static-code` both invoke `devtool check tsc`,
which runs `tsc --noEmit -p end2end/tsconfig.json` and provisions ignored
`end2end/node_modules`. `static-code` also owns unrelated code checks,
TypeScript formatting and `e2eCensusContract.ts`; it has no TypeScript-specific
Nix verdict.

The Nix `e2ePackage` is built from and copies the complete `end2end/` tree. A
narrower direct source filter for tsc therefore does not eliminate that
transitive dependency: another `end2end` change can still invalidate the TS
leaf. Nix request duration and store reuse are not tsc execution duration; a
reused result may mean tsc did not run for the current request.

## Decision

Propose a `static-typescript` Nix leaf that invokes the existing named
`devtool check tsc` definition, with a direct source filter for the project
config, compiler-discovered TS/declaration sources/imports and package/lock
inputs, and the existing pinned Node/TypeScript, `devtool`, `e2ePackage` type
closure and Nix-matched Playwright path. Keep `e2ePackage` unchanged and accept
its whole-`end2end` transitive invalidation; do not add dependency-pruning
machinery.

Add a `typescript` member to devtool's static-check groups, containing `tsc`;
remove only `tsc` from `code`. The named `devtool check tsc` selector remains
valid. Preserve the existing `--all` population and order exactly, with tsc
still between `byte-compile` and `cargo-deny`, and no duplicate entry. Nix
validation runs `static-docs`, `static-typescript`, then `static-code`; the
latter retains its other checks. The existing xtask validation consumer adds a
distinct TS verdict. Hook routing, workflow topology and GitHub required-check
selection do not change.

This narrowly supersedes ADR-0178's **two-group (`docs`/`code`) partition** and
**two-leaf (`static-docs`/`static-code`) layout** by adding one `typescript`
group and one `static-typescript` leaf. The docs/code/code-check meaning and
`devtool` command-definition ownership remain; `static-code` keeps checks that
still consume end2end source. The dedicated leaf therefore isolates the TS
verdict, not every end2end input from all other derivations.

Measure only the `tsc` child with a monotonic clock inside the existing
`devtool check tsc` path and emit one bounded structured stderr timing event
(check name, elapsed milliseconds, outcome) after the child exits. The host
consumer parses this event from that check's current child-process output. The
Nix consumer may report current tsc execution only when it observes the event in
the live output stream of the current `nix build` request and can attribute it
to the builder running. It must never recover current execution timing from a
retained build log or `nix log`; if current-request correlation is not positive,
execution duration is unavailable. Existing host whole-step duration, Nix
request duration, derivation identity and reuse/realization classification stay
separate. Do not add a generic timing framework, telemetry sink or force-build
mechanism.

## Consequences

The named check gains an independent Nix verdict and timing when current tsc
execution is positively observed. A reused/substituted output has no current
execution duration without current builder evidence; historical timing is not
current-request timing. Changes outside the direct TS source filter but inside
`e2ePackage` may still invalidate this leaf, an accepted consequence. The
`--all` inventory/order and host named-check behavior remain stable; the Nix
static catalog becomes three groups and three leaves. Nix never invokes
host-only `xtask`.

This is a proposed decision, not implementation authority. ADR numbering and
acceptance remain owned by post-merge promotion.
