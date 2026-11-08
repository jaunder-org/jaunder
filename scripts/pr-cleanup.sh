#!/usr/bin/env bash
# Only a caller that has positively established unmanaged ownership may invoke
# this operation. This assertion is not runtime ownership discovery or authority.
set -euo pipefail
number=$1 expected_head=$2 ownership=$3
[[ "$ownership" == unmanaged ]] || { echo 'Cleanup requires positively verified unmanaged ownership; preserve managed/unknown checkout' >&2; exit 1; }
[[ "$number" =~ ^[1-9][0-9]*$ && "$expected_head" =~ ^[0-9a-f]{40}$ ]] || { echo 'Expected explicit PR number and full head SHA' >&2; exit 1; }
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
work=$(mktemp -d)
step=pr-cleanup-precheck
trap 'code=$?; if (( code != 0 )); then printf "pr-cleanup failed at %s; preserve remaining state and inspect before retry\n" "$step" >&2; fi; rm -r -- "$work"' EXIT
printf '%s\n' "$step" >&2
gh pr view "$number" --repo jaunder-org/jaunder --json number,state,baseRefName,headRefName,headRefOid,mergedAt,mergeCommit > "$work/pr.json"
jq -e --argjson number "$number" --arg head "$expected_head" '
    .number == $number and .state == "MERGED" and .baseRefName == "main" and .headRefOid == $head
    and (.headRefName | type == "string" and length > 0)
    and (.mergedAt | type == "string" and length > 0)
    and (.mergeCommit.oid | type == "string" and test("^[0-9a-f]{40}$"))
    or error("cleanup requires exact merged PR/head, base main and complete merge evidence")' "$work/pr.json" > /dev/null
branch=$(git branch --show-current)
head=$(git rev-parse --verify HEAD)
status=$(git status --porcelain=v1 --untracked-files=all)
remote_branch=$(jq -r '.headRefName' "$work/pr.json")
[[ -n "$branch" && "$branch" != main && "$branch" == "$remote_branch" && "$head" == "$expected_head" && -z "$status" ]] || { echo 'Cleanup refuses protected local main, detached/wrong branch, divergent head or dirty tree' >&2; exit 1; }
step=fetch-origin; printf '%s\n' "$step" >&2
git fetch origin
step=verify-origin-main; printf '%s\n' "$step" >&2
git merge-base --is-ancestor "$expected_head" origin/main
step=detach-origin-main; printf '%s\n' "$step" >&2
git checkout --detach origin/main
step=delete-local-branch; printf '%s\n' "$step" >&2
git branch -d -- "$branch" >&2
step=cargo-clean; printf '%s\n' "$step" >&2
cargo clean
jq '{outcome:"cleaned",pr:.number,head_sha:.headRefOid,head_ref:.headRefName,merge_commit:.mergeCommit.oid,merged_at:.mergedAt}' "$work/pr.json"
