#!/usr/bin/env bash
# Shared cargo-mutants invocation for this repo. Sourced by ci-shard.sh (the
# scheduled scan) and verify.sh (the local per-file check) so CI and a laptop
# cannot disagree — they must run the tool identically or they answer different
# questions about the same mutant.
#
# Every setting here was learned by getting a wrong answer, not an error.
# Do not drop one because a run "seems to work without it".

# --- Scratch space -----------------------------------------------------------
# cargo-mutants copies the whole source tree per job and builds it there. The
# default TMPDIR is /tmp, a 16 GB tmpfs. A workspace-wide build is far bigger
# than a package one; two jobs of it exhausted the tmpfs and killed a 51-minute
# run at mutant 70 of 71 with ENOSPC. The big disk has ~500 GB.
export TMPDIR="${TMPDIR_MUTANTS:-$HOME/.cache/cargo-mutants-tmp}"
mkdir -p "$TMPDIR"

# --- Which tests count -------------------------------------------------------
# The postgres tests need a live PostgreSQL. Whether one is running is the only
# thing that decides this, so it is a variable rather than a fact:
#
#   MUTANTS_WITH_POSTGRES=1  a cluster is provisioned (CI runs the shard under
#                            `devtool pg run`, which exports JAUNDER_PG_TEST_URL
#                            for a throwaway one). Every test counts, and
#                            storage/src/postgres is mutated like any other code.
#   unset / 0                a bare workstation. The postgres tests are filtered
#                            out AND storage/src/postgres is excluded from
#                            mutation — the two must move together, because
#                            mutating code whose only tests were just filtered
#                            away reports every mutant as MISSED. That is not a
#                            finding, it is noise with a survivor's name on it.
#
# What gets filtered in the no-cluster case:
#
#   postgres       — the case_2_postgres / Backend__Postgres twins. The (?i) is
#                    load-bearing: plain `postgres` misses
#                    `backend_2_Backend__Postgres`.
#   backup_interop — backup_round_trips_full_cycle_across_backends calls
#                    unique_postgres_url() directly, so its NAME never says
#                    postgres. Name-based filtering cannot find it; it must be
#                    named.
#
# One un-excluded postgres test out of 898 fails the unmutated baseline without a
# cluster, and a failed baseline makes cargo-mutants exit 4 having tested nothing
# — which looks like an empty result, not an error. That cost three whole
# packages once and the jaunder package three times.
if [ "${MUTANTS_WITH_POSTGRES:-0}" = "1" ]; then
  MUTANTS_FILTER='all()'
  MUTANTS_PATH_EXCLUDES=()
else
  MUTANTS_FILTER='not test(/(?i)postgres|backup_interop/)'
  MUTANTS_PATH_EXCLUDES=(--exclude 'storage/src/postgres/**')
fi

# --- How to run it -----------------------------------------------------------
# --test-tool nextest
#     Required. Under plain `cargo test` the host crate's metrics tests share
#     one process and a global recorder, so the unmutated baseline fails and the
#     whole package is skipped. nextest gives each test its own process, which
#     is what the repo's own gate uses.
#
# --test-workspace true
#     Required, and the subtlest of the lot. Several crates gate real code
#     behind a default-OFF Cargo feature (common's `sanitize`, its `sqlx`).
#     Package-scoped, nothing enables them, so that code is never compiled —
#     and mutating uncompiled code changes nothing, every test passes, and the
#     mutant is filed as MISSED. common/src/render.rs reported 27 survivors
#     package-scoped and 0 workspace-scoped. flake.nix makes the same point
#     about its doctests derivation: "--workspace is load-bearing, not
#     incidental".
#
# --jobs 2
#     Discovery competing with the repo's gate for disk and CPU already caused
#     one false gate failure (tools-test failed during discovery, passed in 25s
#     after). A false red is dangerous here: the loop's rules say revert-and-
#     skip, so it would throw away good work.
# --timeout 300
#     cargo-mutants auto-sets the timeout from the unmutated baseline's test
#     time — 2s here, giving a 20s cap. That is far too tight once mutants run
#     two at a time: the same builds that take 18-28s clean take 49-91s under
#     load, and the test phase stretches with them. The result was a flood of
#     false timeouts — 83 of common's 507 examined mutants and 159 of storage's
#     497, every one showing exactly "20s test", on mundane mutants like
#     `replace > with ==` in a FromStr that cannot possibly hang.
#
#     This one is worse than a slow run: a timed-out mutant is UNEXAMINED. It is
#     neither caught nor missed, so it silently shrinks the scan's real coverage
#     while the summary still reads "0 missed".
#
#     300s is deliberately generous. A mutant that genuinely loops forever costs
#     five minutes once; a false timeout costs a hole in the results.
#
# MUTANTS_JOBS overrides the parallelism. It exists for CI: a hosted runner has
# far fewer cores than a workstation, and two jobs there means two builds and two
# workspace test runs competing for the same handful of cores — which is how the
# timeout problem above was created in the first place. Leave it alone locally.
run_mutants() {
  cargo mutants \
    --jobs "${MUTANTS_JOBS:-2}" \
    --no-shuffle \
    --test-tool nextest \
    --test-workspace true \
    --timeout 300 \
    "${MUTANTS_PATH_EXCLUDES[@]}" \
    "$@" \
    -- -E "$MUTANTS_FILTER"
}
