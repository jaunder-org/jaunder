#!/usr/bin/env bash
# Shared end-to-end test fixture seeding.
#
# Creates the user accounts and clears the mail capture file that the
# Playwright e2e suite expects. Invoked from `end2end/run-e2e.sh` (the
# cargo-leptos local path) and `flake.nix` (the NixOS VM e2e checks) so
# the two environments stay in sync.
#
# Assumes the caller has already reset the underlying database to a
# clean state (local & Nix-SQLite: stop-the-server + wipe data dir + restart,
# letting jaunder auto-init recreate the schema; Nix-Postgres: TRUNCATE in
# the calling Python). This script never enumerates tables, so it doesn't
# need updating when the schema grows.
#
# Backend-agnostic: only goes through `jaunder` CLI + the mail file. The
# caller is responsible for any backend-specific seed steps (e.g. setting
# `site.registration_policy` via the appropriate SQL client) since there
# is no CLI for that yet, and for pointing the jaunder binary at the right
# backend via JAUNDER_DB / JAUNDER_STORAGE_PATH env vars (which `jaunder`
# already reads).
#
# Required:
#   JAUNDER_BIN                 path to the jaunder binary
#   JAUNDER_MAIL_CAPTURE_FILE   path to the mail capture jsonl file
#
# JAUNDER_DB / JAUNDER_STORAGE_PATH may be set in the environment for
# the underlying jaunder invocation — those are read by the binary
# directly.
set -euo pipefail

: "${JAUNDER_BIN:?must be set to the jaunder binary path}"
: "${JAUNDER_MAIL_CAPTURE_FILE:?must be set}"

"$JAUNDER_BIN" user-create --username testlogin --password testpassword123
"$JAUNDER_BIN" user-create --username testnoemail --password testpassword123

rm -f "$JAUNDER_MAIL_CAPTURE_FILE"
