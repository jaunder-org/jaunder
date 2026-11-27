# Issue #2146: kache hit and miss counts on Cargo steps

## Outcome

Each compiling host Cargo step a `dev` gate runs under kache records, on its
step span, how many compiles kache served from its cache, how many it compiled,
and kache's estimate of the compile work it avoided. A retrospective can then
tell how much compilation the cache spared each Cargo step. Tracing still never
changes a step's outcome.

## Approach and tradeoffs

kache 2.1.1 (the devShell's pinned version) can already report this. Each rustc
call through the wrapper appends an event to the per-user log, labelled with a
build **root** (kache's "build tree") and a **session id**.
`kache stats --json --last-build --root R` reports the latest session of tree
`R`, with `summary.local_hits`, `prefetch_hits`, `remote_hits`, `misses` and
`time_saved_ms`. By default the tree is the Cargo workspace, and one session
covers every Cargo command in it until it has been idle for five minutes. So a
default query merges consecutive steps, and builds in the same checkout that run
at the same time. kache's documented `KACHE_EVENT_ROOT` variable replaces the
tree on the events and does not enter cache keys.

The change works like this:

1. The compiling Cargo steps opt in where they are defined: the six Clippy
   leaves, `tools-tests-build`, `tools-tests`, `image-worker`, `nextest-build`,
   `nextest` (`pg-run cargo nextest`) and `csr-wasm`. `cargo fmt` and
   `cargo deny` compile nothing and do not opt in. An opted-in step that runs
   with `RUSTC_WRAPPER=kache` (host gates; `checkout.enter` sets it) gets
   `KACHE_EVENT_ROOT` set to a path unique to that step run. The path is under
   the checkout, is built from the trace id and the step's span id, and is never
   created on disk.
2. Once the child exits, still inside the step span and unless the step was
   signalled, `dev` runs `kache stats --json --last-build --root <that path>`.
   This query is not a step of its own, like the Nix queries behind
   `jaunder.cache`. From its output `dev` records:
   - `jaunder.kache.hits` = local + prefetch + remote hits;
   - `jaunder.kache.misses`;
   - `jaunder.kache.avoided_compile_ms` = kache's `time_saved_ms`. This is
     summed compile work, not wall time: parallel hits can add up to more than
     the step took.
3. Zeros are valid. A step whose compiler work was only rustc probes records
   `0`. Nothing is recorded, and the step's status and exit code stay as the
   child left them, in these cases: the query fails for any reason (nonzero
   exit, for instance when there are no events; missing kache; malformed JSON;
   fields absent or not integers), the step was signalled, or
   `KACHE_DISABLED=1`.

Sandboxed (Nix) runs set no `RUSTC_WRAPPER`, so they get no variable, no query
and no attributes.

Probe (2026-11-27, this checkout):
`cargo check --manifest-path tools/Cargo.toml -p traced-run` under root
`probe-a` reported 27 hits and 22 misses in its own session. A second run under
root `probe-b` got a different session and 1 hit, 0 misses. That hit was the
crate compiled under `probe-a`, so the root does not change cache keys. Queries
took 0.2-0.6 s. `kache targets --json` showed no phantom workspaces, so kache
tracks target directories, not event roots.

Alternatives rejected:

- **A stable root per (checkout, step name).** It would keep `kache explain`
  baselines. But a gate re-run within five minutes would merge into the earlier
  session and record that run's compiles as well: wrong counts.
- **Query by time window** (`--since`, root = checkout). The window can only be
  set to whole seconds, and it would mix in other builds in the same checkout.
- **Read `events.jsonl` directly.** Its format is kache's internal one (schema
  27); the stats JSON is the interface kache documents.
- **Detect Cargo steps from argv.** It would miss `pg-run cargo …` or match any
  argument named `cargo`. The call sites are few, and naming them makes the set
  of steps exact.

Accepted costs and limitations:

- **Query time.** Each opted-in step takes about 0.2-0.6 s longer. A full host
  `dev check` runs about 12 such steps, so 2-7 s. kache rotates `events.jsonl`
  by size, which bounds the growth.
- **Less from kache's own tools.** Each step run is its own build tree to kache:
  - `kache explain` and the wrapper's recorded key diff find no same-tree
    earlier build for a `dev` step;
  - `kache stats --last-build --root <checkout>` returns only the latest step;
  - the monitor lists each step as its own build. This departs from ADR-0256's
    "everything else is kache's default".
- **Accuracy.** The counts are only as complete as kache's event log; for
  example, rotation in the middle of a step can drop its first events.
- **kache upgrades** mean checking these JSON fields again. An unexpected shape
  records nothing rather than wrong values.

## Acceptance

- Pytest tests (100% branch coverage as `dev-python` requires) use a fake
  `cargo`. The suite's default fake `kache` fails without logging, so existing
  command-sequence tests are unchanged and no test reaches the real kache store.
  A configurable fake report then shows that:
  - an opted-in host step's child sees a `KACHE_EVENT_ROOT` unique to its step,
    and the span carries the three attributes taken from the report, zeros
    included;
  - when kache fails, prints malformed JSON or omits fields, the step records no
    kache attributes and keeps the child's status and exit code;
  - a step that did not opt in, and a sandboxed opted-in step, get neither the
    variable nor a query;
  - a signalled opted-in step runs no query and records no kache attributes.
- CONTRIBUTING "Observation and traces" documents the three attributes: which
  steps carry them, what zero means, and when they are absent. The CONTRIBUTING
  kache paragraph ("Beyond that environment kache runs on its defaults") and the
  matching `docs/ARCHITECTURE.md` passages name the per-step `KACHE_EVENT_ROOT`
  and its cost to `kache explain`.
- An ADR draft (`jaunder-adr`) records the per-step build-tree decision and its
  costs. It extends ADR-0254's step accounting and amends ADR-0256's
  kache-defaults statement.
- Execution proof (not committed): a real `dev test-local` run whose
  `image-worker`/`nextest-build` spans in the merged trace carry the attributes.

## Boundaries

- No change to `dev trace summary`, CI workflows or kache configuration, apart
  from the one per-step environment variable.
- No data for Nix-sandboxed Cargo builds; per-step totals only, no per-crate
  detail.
