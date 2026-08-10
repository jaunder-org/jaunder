#!/usr/bin/env bash
# Shared cargo-mutants invocation for this repo. Sourced by discover.sh and
# verify.sh so discovery and verification cannot disagree — they must run the
# tool identically or they answer different questions about the same mutant.
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
# Everything excluded needs a live PostgreSQL that is not running here. Every
# excluded test has a sqlite twin covering the same code, so no mutant goes
# unexamined.
#
#   postgres       — the case_2_postgres / Backend__Postgres twins. The (?i) is
#                    load-bearing: plain `postgres` misses
#                    `backend_2_Backend__Postgres`.
#   backup_interop — backup_round_trips_full_cycle_across_backends calls
#                    unique_postgres_url() directly, so its NAME never says
#                    postgres. Name-based filtering cannot find it; it must be
#                    named.
#
# One un-excluded postgres test out of 898 fails the unmutated baseline, and a
# failed baseline makes cargo-mutants exit 4 having tested nothing — which looks
# like an empty result, not an error. That cost three whole packages once and
# the jaunder package three times.
MUTANTS_FILTER='not test(/(?i)postgres|backup_interop/)'

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
run_mutants() {
  cargo mutants \
    --jobs 2 \
    --no-shuffle \
    --test-tool nextest \
    --test-workspace true \
    "$@" \
    -- -E "$MUTANTS_FILTER"
}
