# Reuse tooling dependencies through the static-check path (#1752)

## Outcome

Repeated tooling/static-check work reuses unaffected compiled dependencies
across source edits and existing checkouts. Deliver a reproducible improved
Crane baseline for #1754, or an evidence-backed no-change disposition if the
selected hypothesis is disproved. Start with tools-clippy and its tooling
dependency path.

## Load-bearing decisions

- Preserve the delivered #1734 Nix-selected host-tool launcher, runtime checkout
  discovery, source admission and qualification provenance contracts.
- Nix derivation/output identity and Cargo fingerprints remain cache
  authorities; do not introduce a source fingerprint ledger or a custom cache
  service.
- Preserve `.#devtool`'s release profile, default performance/system-artifacts
  features, pinned compiler, native inputs and existing test-authority split.
  Qualification and documentation tools retain their distinct capabilities.
- Preserve the shared tools-clippy definition, all-targets population and
  warnings-as-errors. Host and sandbox callers still use that single definition.
- Preserve the complete static-code verdict population and its documentation
  partition (ADR-0178). Intermediate compilation may be cached independently;
  this does not create a smaller authoritative static verdict.
- Preserve workspace-specific offline Cargo homes and hermetic sandbox
  execution. xtask executes only on the host; independent tools/xtask tests
  remain required.
- Dependency preparation must export nonempty artifacts from the target actually
  compiled; consumers import them into the target they actually use. A suspected
  target-directory mismatch is a diagnosis to prove, not an assumed fix.
- Dependency-only preparation may use generated dummy sources, but real consumer
  sources retain every required manifest/lock/configuration/build-script,
  embedded/generated input, local path dependency and native dependency.
- Artifact reuse must match compiler, target, profile, features, flags and
  native inputs. Release binary artifacts are not assumed to seed
  dev/all-targets clippy. A mismatch must rebuild the affected work, never
  report a stale successful check.
- Shared preparation changes preserve ordinary, documentation and qualification
  consumers. Do not broaden feature sets or weaken source admission for reuse.
- Generic package/source wiring remains Darwin-compatible. Linux-only static
  authority stays scoped as documented; x86_64-linux measurements do not prove
  macOS behavior or cross-platform artifact reuse.

## Acceptance

1. Trace tools-clippy from its supported developer command through tool
   selection, source/vendor preparation, compilation artifacts and verdict.
   Record effective phase directories, setup/configure hooks,
   profiles/features/flags and native inputs. Distinguish source vendoring,
   compiled reuse and substituted outputs.
2. Prove suspected export/import behavior using existing archive/log evidence or
   a tiny dependency-free reproduction with the same pinned Crane hooks before
   paying for a full production graph. Retain both confirming and disproving
   data.
3. Completion has two branches. An improved candidate repairs the narrowest
   confirmed source/artifact seam and demonstrates correct nonempty
   export/import and real reuse; another preparation layer alone is not
   delivery. A justified no-change disposition instead retains
   baseline/diagnostic and real-consumer evidence that disproves the hypothesis
   or explains why a correct repair offers no benefit, names remaining costs,
   and hands #1754 the unchanged baseline. Requirements 1–2 and the protocol
   safeguards govern both branches; changed boundary regressions, candidate
   comparisons and lower-cost proof apply only when an improved candidate
   exists.
4. An unrelated source edit preserves unaffected dependency artifact identities;
   a relevant source edit invalidates its real consumer. Manifest/lock, build
   scripts, embedded inputs, compiler flags and native dependencies invalidate
   the appropriate outputs. Do not exclude required inputs to manufacture reuse.
5. Add maintained regression proof of the changed boundary and actual artifact
   consumption. Extend existing source-closure probes where suitable; an
   identity comparison alone is not proof that Cargo used an exported archive.
6. Measure the baseline and, when present, a matched improved candidate with the
   agreed protocol: named revision/configuration, warm no-op, docs, leaf
   tooling, shared dependency source, one dependency-feature configuration
   change and owner-operated reuse in a second existing checkout. Record
   before/after resolved feature graphs: the configuration arm must genuinely
   change that graph while preserving required defaults/behavior/populations; an
   already unified feature is invalid. Label configuration invalidation, not a
   version upgrade. Change exactly one input per arm and restore source bytes
   between arms; do not purge shared caches or mutate another owner's checkout.
   Record locally valid output identities before each run. Postpone timing work
   during uncontrolled competing jobs; a granted window alone is not isolation.
7. Respect the measurement ceiling: three no-op samples and at most two distinct
   marker realizations per matched leaf/shared arm, with warm repeats. Reduce
   expensive arms if first-arm pressure warrants. An executing warm no-op must
   show freshness/no recompilation of unaffected dependencies. Classify cached
   final-output lookup separately: it is not compiler-freshness or fresh-verdict
   proof. Preserve unsuccessful arm outcomes.
8. Report repetition count/spread, end-to-end wall time, retained/peak scratch,
   host load/concurrent work and cache state. Attribute evaluation, transfer,
   unpacking, compilation, linking and verdict execution only where observable;
   label unavailable phases and sampled-memory limitations explicitly.
9. An improved candidate demonstrates lower repeated compilation or cost without
   reducing correctness. Both completion branches report setup/unpacking/scratch
   tradeoffs and any disproved hypothesis honestly.
10. Hand #1754 the baseline source revision, exact release
    command/configuration, source/input/output identities, cache description,
    measurements and real consumer proof. Keep its release comparison distinct
    from dev clippy timings.

## Boundaries

No production per-unit build migration, product-cache expansion, gate-population
reduction, custom provenance protocol, deployment or global cleanup. Heavy work
requires separately granted exact windows; design approval does not reserve
them. No new coverage/CRAP/lint suppression without explicit operator approval.

For an improved candidate, commit changed source, maintained regression proofs
and current contract documentation. A no-change disposition commits no
artificial implementation; its conclusions and baseline references belong in the
issue. Logs, measurements, review packets and probes remain ignored/session
evidence. This working spec is deleted after conformance; merge remains a
separate explicit human approval.
