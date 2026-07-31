# Determinism fixtures (#745)

Real `cargo xtask server-fn-coverage regenerate` output from **three distinct
executions of the same e2e derivation on the same tree**. They are the evidence
behind #745's central claim, and `snapshot.rs`'s
`runs_that_disagree_on_titles_still_render_one_compared_snapshot` is what keeps
that claim checkable in milliseconds instead of on trust.

## What they show

The three files disagree — that is the whole point. They disagree **only** in
`covered`'s test-title lists. Their covered fn key sets and their `orphans` maps
are identical, which is why #745 could stop byte-comparing the titles without
weakening what the gate asserts.

The disagreement is a single title,
`owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app`,
moving between runs on `timeline::list_home_feed` and
`posts::get_default_audience_selection`. That test asserts a redirect and ends
while its page is still booting; the boot then issues those calls, and how far
it gets before teardown differs run to run. The attribution is correct — the
requests really are that test's — but it is not reproducible.

## Format

**The pre-#745 combined shape** —
`{"covered": {key: [titles]}, "orphans": {...}}` — which the current code no
longer parses. That is deliberate: they are historical artifacts, so the test
brings its own `CombinedRun` deserialize struct rather than being rewritten into
today's format, which would destroy the evidence.

## Provenance

Produced on `758b5f26` (this branch's base), before the split landed:

```bash
# one baseline build, then three forced re-executions
nix build --accept-flake-config -L .#checks.x86_64-linux.e2e-sqlite-chromium
nix build --rebuild --keep-failed --accept-flake-config -L .#checks.x86_64-linux.e2e-sqlite-chromium   # ×3
# each run's <out>.check/capture-sqlite.tar.gz copied to
# .xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz, then:
cargo xtask server-fn-coverage regenerate
```

Nix reported the derivation "may not be deterministic" on every `--rebuild`,
which is how the differing outputs were obtained at all: on an unchanged tree a
plain re-run replays the cached output in ~4 s and cannot show the variance.

There were **four** runs; two produced byte-identical snapshots, hence three
files. `run-c.json` is the one that matched the artifact committed on `main` at
the time.

The captures themselves are not committed — ~2.2 MB compressed each, ~26 MB of
JSONL once extracted, and nothing in the repo reads them.
