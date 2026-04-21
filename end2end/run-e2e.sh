#!/usr/bin/env bash
# Setup script for cargo leptos end-to-end.
#
# cargo-leptos builds the project, starts the server (which auto-initializes
# the SQLite database in dev mode), then runs this script in the end2end
# directory.  We seed the test fixtures and invoke Playwright.
set -euo pipefail

# The jaunder binary built by cargo-leptos lives in the target directory.
BIN="../target/server/debug/jaunder"
if [[ ! -x "$BIN" ]]; then
  BIN="../target/debug/jaunder"
fi

MAIL_CAPTURE_FILE="${JAUNDER_MAIL_CAPTURE_FILE:-/tmp/jaunder-mail.jsonl}"
export JAUNDER_MAIL_CAPTURE_FILE="$MAIL_CAPTURE_FILE"

# Wait for the server to be ready (cargo-leptos may still be starting it).
for i in $(seq 1 30); do
  if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

# Seed registration policy.
sqlite3 ../data/jaunder.db \
  "INSERT OR REPLACE INTO site_config (key, value) VALUES ('site.registration_policy', 'open')"

# Create the test user (ignore failure if it already exists).
"$BIN" user-create --username testlogin --password testpassword123 2>/dev/null || true

# Run Playwright — chromium only for local dev.
playwright test --project chromium
