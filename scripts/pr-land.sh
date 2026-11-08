#!/usr/bin/env bash
# Invocation requires prior recorded human approval for this exact PR/head/scope.
# Recovery additionally requires considered one-shot re-enqueue authorization.
set -euo pipefail
number=$1 expected_head=$2 mode=${3:-ordinary}
[[ "$number" =~ ^[1-9][0-9]*$ && "$expected_head" =~ ^[0-9a-f]{40}$ ]] || { echo 'Expected explicit PR number and full head SHA' >&2; exit 1; }
[[ "$mode" == ordinary || "$mode" == --reenqueue ]] || { echo 'Expected optional --reenqueue' >&2; exit 1; }
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
# The observer infers its repository from origin; never act in a different repo.
origin=$(git remote get-url origin)
[[ "$origin" == https://github.com/jaunder-org/jaunder.git || "$origin" == https://github.com/jaunder-org/jaunder || "$origin" == git@github.com:jaunder-org/jaunder.git ]] || { echo 'Land requires jaunder-org/jaunder origin' >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -r -- "$work"' EXIT
read_pr() {
    gh api graphql -f query='query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){pullRequest(number:$number){number state baseRefName headRefName headRefOid isInMergeQueue autoMergeRequest{enabledAt} mergedAt mergeCommit{oid}}}}' -F number="$number" > "$work/live.json"
    jq -e --argjson number "$number" --arg head "$expected_head" '
        (.errors // [] | length == 0) and (.data.repository.pullRequest |
        .number == $number and .headRefOid == $head and .baseRefName == "main"
        and (.headRefName | type == "string" and length > 0)
        and (.isInMergeQueue | type == "boolean")
        and (has("autoMergeRequest") and (.autoMergeRequest == null or
            (.autoMergeRequest | type == "object" and (.enabledAt | type == "string" and length > 0))))
        and (.state == "OPEN" or .state == "MERGED"))
        or error("land requires exact live PR/head, base main and complete queue evidence")' "$work/live.json" > /dev/null
}
observe_once() {
    local code=0
    just pr-watch "$number" --once --expected-head "$expected_head" "$@" > "$work/preflight.json" || code=$?
    # Exit 1 carries an actionable observer verdict; setup/transport absence is
    # never permission to arm. Validate identity and use only the shared verdict.
    (( code <= 1 )) || return "$code"
    jq -e --argjson number "$number" --arg head "$expected_head" --argjson code "$code" '
        .pr == $number and .head_sha == $head and (.outcome | type == "string")
        and (if .outcome == "ready-to-land" or .outcome == "merged" then $code == 0 else $code == 1 end)
        or error("inconsistent observer identity/status; refusing action")' "$work/preflight.json" > /dev/null
}
read_pr
branch=$(git branch --show-current)
head=$(git rev-parse --verify HEAD)
remote_branch=$(jq -r '.data.repository.pullRequest.headRefName' "$work/live.json")
[[ "$branch" != "$remote_branch" || "$head" == "$expected_head" ]] || { echo 'Local PR branch head diverges from approved head' >&2; exit 1; }
ack=()
observe_once
if [[ "$mode" == --reenqueue ]]; then
    jq -e '.data.repository.pullRequest | .state == "OPEN" and .isInMergeQueue == false and .autoMergeRequest == null' "$work/live.json" > /dev/null
    historical=$(jq -er 'select(.outcome == "ejected") | .pointer | select(type == "string" and test("^https://github.com/jaunder-org/jaunder/actions/runs/[0-9]+$"))' "$work/preflight.json")
    ack=(--acknowledged-ejection "$historical")
    # Ejection alone is not readiness: prove readiness with only that historical
    # run acknowledged, using the SAME observer, before the sole native action.
    observe_once "${ack[@]}"
fi
jq -e '.outcome == "ready-to-land"' "$work/preflight.json" > /dev/null
# Final live identity read narrows the preflight/action race; GitHub owns the
# atomic head lease. No retry, re-arm or mutation fallback on any failure/no-op.
read_pr
jq -e '.data.repository.pullRequest | .state == "OPEN" and .isInMergeQueue == false and .autoMergeRequest == null' "$work/live.json" > /dev/null
gh pr merge "$number" --repo jaunder-org/jaunder --auto --merge --match-head-commit "$expected_head" >&2
read_pr
jq -e '.data.repository.pullRequest | .isInMergeQueue == true or .autoMergeRequest != null or (.state == "MERGED" and (.mergedAt | type == "string" and length > 0) and (.mergeCommit.oid | type == "string" and test("^[0-9a-f]{40}$"))) or error("merge action did not establish arm, queue or complete merged evidence; inspect, do not retry")' "$work/live.json" > /dev/null
printf 'Native action readback verified; observing approved head (not a merge verdict)\n' >&2
queue_history=()
# read_pr already proved this is the expected head. Carry only positive queue
# membership so disappearance before observer establishment is not forgotten.
if jq -e '.data.repository.pullRequest.isInMergeQueue == true' "$work/live.json" > /dev/null; then
    queue_history=(--observed-queued-head "$expected_head")
fi
code=0
just pr-watch "$number" --until merged --expected-head "$expected_head" "${ack[@]}" "${queue_history[@]}" > "$work/final.json" || code=$?
# Preserve the observer's direct report and failure status. Only complete merged
# evidence for this head is success; readiness/arming/queueing are not merging.
cat "$work/final.json"
(( code == 0 )) || exit "$code"
jq -e --argjson number "$number" --arg head "$expected_head" '.pr == $number and .head_sha == $head and .outcome == "merged" and (.pointer | type == "string" and test("^[0-9a-f]{40}$")) and (.detail | type == "string" and length > 0)' "$work/final.json" > /dev/null
