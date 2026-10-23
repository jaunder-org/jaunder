# Keep the existing tooling root population independent of TOML pretty-printing.
# Only [workspace].members is changed; all other tables and arrays pass through.
set -euo pipefail
manifest=$1
awk '
  /^[[:space:]]*\[/ {
    workspace = ($0 ~ /^\[workspace\][[:space:]]*$/)
    members = 0
  }
  workspace && /^members[[:space:]]*=[[:space:]]*\[[[:space:]]*$/ { members = 1 }
  members && /^[[:space:]]*"(client|csr|server|test-support|web)",?[[:space:]]*$/ { next }
  { print }
  members && /^[[:space:]]*\][[:space:]]*$/ { members = 0 }
' "$manifest" > "$manifest.tooling"
mv "$manifest.tooling" "$manifest"
