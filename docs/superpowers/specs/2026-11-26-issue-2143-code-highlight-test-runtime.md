# Issue #2143 — keep the code_highlight corpus tests inside the nextest timeout

## Outcome

The `host` code-highlight tests that configure every bundled grammar finish well
inside nextest's 30s kill budget under full-workspace `dev check` load. Today
they take about 11s each when run alone. No test loses coverage.

## Measured cost

Isolated runs (`dev test-local -p host -E 'test(/code_highlight/)'`), before:

| Test                                                 | Grammars | Time  |
| ---------------------------------------------------- | -------- | ----- |
| `quality::catalog_corpus_names_every_grammar_…`      | 42       | 11.9s |
| `tests::every_alias_renders_the_same_code_…`         | 40       | 10.8s |
| `tests::every_bundled_grammar_has_a_working_query_…` | 40       | 10.8s |
| `tests::mainstream_languages_emit_semantic_tokens_…` | 15       | 7.2s  |

Time tracks how many grammars a test configures, not how many aliases, formats
or renders it runs. The dominant cost is building each grammar's
`HighlightConfiguration` once per test process. That is inferred from a
crate-scoped probe, not from profiling. The step is mostly Tree-sitter query
compilation in the `tree-sitter` crate's C core, which Cargo's dev profile
builds at `-O0`. Optimizing only that crate removes about 63% of the runtime.
Because nextest runs one process per test, the per-process `OnceLock` caches
never last beyond a single test.

## Approach and tradeoffs

Add `[profile.dev.package.tree-sitter] opt-level = 3` to the workspace
`Cargo.toml`. The test profile inherits dev, so local nextest, CI nextest and
the coverage build (crane passes no `--profile`) all get the override.
`cargo-llvm-cov` instruments only workspace crates, so optimizing this
third-party crate does not change coverage. A probe measured 4.4s / 3.9s / 3.8s
/ 2.6s for the four tests above: about 3× faster, and below even the 5s
slow-warning. The observed load slowdown is about 2.7× (11s alone, over 30s
under load). At that slowdown, the slowest test, 4.4s alone, would take about
12s, well under the 30s limit. Every other dev-profile build also gets the
override, including the debug `jaunder` binary's Nix dependency artifacts. That
is harmless, and it slightly speeds up dev-server highlighting.

Alternatives considered:

- **Per-test nextest timeout override.** This hides the cost rather than
  removing it, and every future grammar makes it worse.
- **Splitting or sharing the corpus work.** One process per test means sharing
  would need a nextest test group or a build-time cache. That is more machinery
  than a profile line. Splitting the tests per grammar multiplies process
  startups and still compiles each query at `-O0`.

Accepted costs: one-time rebuild of `tree-sitter` and its dependents after this
change. The optimized C build takes slightly longer, but it is cached like any
other dependency. Stepping through the Tree-sitter C core in a debugger is less
reliable in dev builds.

## Acceptance

- Isolated timings before and after are recorded in the PR (above table plus the
  post-change run).
- `dev check` passes, and none of these tests times out. The PR records the
  slowest timing that run's nextest output shows for these tests, so the margin
  can be checked.
- Test code is unchanged, so every grammar and alias is still checked.

## Boundaries

- No changes to the tests, the grammar registry or `.config/nextest.toml`.
- No opt-level changes for other crates. Release and wasm profiles are
  untouched.
