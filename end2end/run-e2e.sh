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

# Fixture setup goes through the test-support binary (links storage — the same
# code path as the flake VM's seed_db). cargo-leptos builds only the server, so
# build test-support here; it lands in the workspace target/debug.
cargo build -p test-support
TEST_SUPPORT="$(git rev-parse --show-toplevel)/target/debug/test-support"
export JAUNDER_DB_PATH="${JAUNDER_DB_PATH:-../data/jaunder.db}"
export JAUNDER_MAIL_CAPTURE_FILE="${JAUNDER_MAIL_CAPTURE_FILE:-/tmp/jaunder-mail.jsonl}"
export JAUNDER_WEBSUB_CAPTURE_FILE="${JAUNDER_WEBSUB_CAPTURE_FILE:-/tmp/jaunder-websub.jsonl}"

# Wait for the server to be ready (cargo-leptos may still be starting it).
for _ in $(seq 1 30); do
    if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

# Seed fixtures through test-support (links storage; same code path as the
# flake VM's seed_db). The local backend is always SQLite, so JAUNDER_DB points
# at the SQLite file the server auto-initialised. These must match flake.nix's
# seed_db: three fixture users, open registration so the auth-flow tests can
# register fresh users, a WebSub hub URL so feeds.spec.ts hub-ping tests have a
# hub to ping, and a reset mail-capture file.
export JAUNDER_DB="sqlite:$JAUNDER_DB_PATH"
"$TEST_SUPPORT" create-user --username testlogin --password testpassword123
"$TEST_SUPPORT" create-user --username testnoemail --password testpassword123
"$TEST_SUPPORT" create-user --username testoperator --password testpassword123 --operator
"$TEST_SUPPORT" set-site-config --key site.registration_policy --value open
"$TEST_SUPPORT" set-site-config --key feeds.websub_hub_url --value https://hub.test.local/
"$TEST_SUPPORT" reset-mail --path "$JAUNDER_MAIL_CAPTURE_FILE"

# Run Playwright — chromium only for local dev.
# To re-enable verbose server logging, run with JAUNDER_VERBOSE=true or pass --verbose to the server.
playwright test --project chromium --workers=1
