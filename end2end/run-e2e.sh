#!/usr/bin/env bash
# Setup script for cargo leptos end-to-end.
#
# cargo-leptos builds the project, starts the server (which auto-initializes
# the SQLite database in dev mode), then runs this script in the end2end
# directory. We seed the test fixtures via the shared script and invoke
# Playwright.
#
# Run this via `scripts/e2e-local.sh` rather than `cargo leptos end-to-end`
# directly — the wrapper provides a per-run temp storage dir via env vars
# (JAUNDER_STORAGE_PATH, JAUNDER_DB, JAUNDER_DB_PATH, JAUNDER_MAIL_CAPTURE_FILE)
# so the seed script can run against a clean schema. Falling back to the
# repo-relative dev paths preserves the legacy direct-invocation flow.
set -euo pipefail

# The jaunder binary built by cargo-leptos lives in the target directory.
BIN="../target/server/debug/jaunder"
if [[ ! -x "$BIN" ]]; then
    BIN="../target/debug/jaunder"
fi
export JAUNDER_BIN="$BIN"
export JAUNDER_DB_PATH="${JAUNDER_DB_PATH:-../data/jaunder.db}"
export JAUNDER_MAIL_CAPTURE_FILE="${JAUNDER_MAIL_CAPTURE_FILE:-/tmp/jaunder-mail.jsonl}"

# Wait for the server to be ready (cargo-leptos may still be starting it).
for _ in $(seq 1 30); do
    if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

# Open registration so the auth-flow tests can register fresh users. The
# shared seed script is backend-agnostic and doesn't touch site_config;
# we do it here via sqlite3 (the local path is always SQLite).
sqlite3 "$JAUNDER_DB_PATH" \
    "INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')"

"$(git rev-parse --show-toplevel)/scripts/seed-e2e-fixtures.sh"

# Run Playwright — chromium only for local dev.
# To re-enable verbose server logging, run with JAUNDER_VERBOSE=true or pass --verbose to the server.
playwright test --project chromium
