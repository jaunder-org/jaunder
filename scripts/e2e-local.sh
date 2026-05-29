#!/usr/bin/env bash
# Run `cargo leptos end-to-end` against a per-run temp storage dir.
#
# Without this wrapper the jaunder server uses `./data/jaunder.db`, which
# persists across runs and accumulates state (modified test-user passwords,
# leftover sessions, stale email_verifications, etc.). That accumulation
# was causing five tests to fail locally while passing under
# `scripts/verify` (which runs each e2e suite in a fresh NixOS VM).
#
# By exporting JAUNDER_STORAGE_PATH and JAUNDER_DB the server's auto-init
# creates a fresh schema in the temp dir. The dir is removed on exit so
# back-to-back runs always start clean.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

TMPDIR="$(mktemp -d -t jaunder-e2e.XXXXXX)"
cleanup() {
    rm -rf "$TMPDIR"
}
trap cleanup EXIT INT TERM

export JAUNDER_STORAGE_PATH="$TMPDIR/storage"
export JAUNDER_DB="sqlite:$TMPDIR/jaunder.db"
export JAUNDER_DB_PATH="$TMPDIR/jaunder.db"
export JAUNDER_MAIL_CAPTURE_FILE="$TMPDIR/mail.jsonl"
export JAUNDER_WEBSUB_CAPTURE_FILE="$TMPDIR/websub.jsonl"
mkdir -p "$JAUNDER_STORAGE_PATH"

exec cargo leptos end-to-end "$@"
