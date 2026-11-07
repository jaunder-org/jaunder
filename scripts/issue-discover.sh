#!/usr/bin/env bash
# Read-only discovery. gh owns pagination; jq checks complete evidence before
# classification. Nothing here claims an issue or acquires runtime ownership.
set -euo pipefail
source scripts/issue-metadata.sh
resolve_milestone "$1"
issue_list "repos/$repo/issues?state=open&milestone=$milestone_number&per_page=100" > "$work/issues.json"
jq -ce 'map(if has("pull_request") then
    if (.pull_request | type) == "object" then empty else error("invalid PR discriminator") end
    elif (.title | type) == "string" and (.html_url | type) == "string"
      and (.labels | type) == "array" and all(.labels[]; (.name | type) == "string") then .
    else error("malformed issue inventory") end)' "$work/issues.json" > "$work/observed.json"
: > "$work/candidates.jsonl"
: > "$work/skipped.jsonl"
jq -c '.[]' "$work/observed.json" > "$work/rows.jsonl"
while IFS= read -r issue; do
    number=$(jq -r '.number' <<< "$issue")
    gh api graphql -F "number=$number" -f 'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){issue(number:$number){projectItems(first:100){nodes{id project{id} status:fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}} pageInfo{hasNextPage}}}}}' > "$work/project.json"
    status=$(jq -er --arg project "$project" '
        if (.errors // [] | length) == 0 and .data.repository.issue.projectItems.pageInfo.hasNextPage == false
        then .data.repository.issue.projectItems.nodes else error("incomplete project inventory") end
        | if type == "array" and all(.[]; (.id | type == "string" and length > 0) and (.project.id | type == "string" and length > 0)) then . else error("malformed project item") end
        | map(select(.project.id == $project))
        | if length == 0 then "" elif length == 1 then
            if (.[0] | has("status") | not) then error("missing status evidence") elif .[0].status == null then "" elif (.[0].status.name | type) == "string" then .[0].status.name else error("invalid status") end
          else error("ambiguous Backlog item") end' "$work/project.json")
    issue_list "repos/$repo/issues/$number/dependencies/blocked_by?per_page=100" > "$work/dependencies.json"
    blockers=$(jq -ce 'map(if .state == "open" then .number elif .state == "closed" then empty else error("invalid blocker state") end) | sort' "$work/dependencies.json")
    reason=''
    if [[ "$status" == 'In Progress' ]]; then reason=claimed
    elif [[ -z "$status" ]]; then reason=missing-project-status
    elif [[ "$blockers" != '[]' ]]; then reason=blocked
    fi
    if [[ -n "$reason" ]]; then
        jq -cn --argjson issue "$issue" --arg reason "$reason" --argjson blocked_by "$blockers" \
            '$issue | {number,title,reason:$reason,blocked_by:$blocked_by}' >> "$work/skipped.jsonl"
        continue
    fi
    # Refuse ambiguous resume evidence instead of picking an arbitrary branch/doc.
    branches=$(git for-each-ref '--format=%(refname:short)' "refs/heads/issue-$number-*")
    branch=$(jq -Rn --arg lines "$branches" '$lines | split("\n") | map(select(length > 0)) | if length <= 1 then .[0] else error("ambiguous issue branches") end')
    spec_paths='' plan_paths=''
    if [[ -d docs/superpowers/specs ]]; then
        spec_paths=$(find docs/superpowers/specs -maxdepth 1 -type f \( -name "*issue-$number-*.md" -o -name "*issue-$number.md" \))
    elif [[ -e docs/superpowers/specs ]]; then echo 'Specs path is not a directory' >&2; exit 1; fi
    if [[ -d docs/superpowers/plans ]]; then
        plan_paths=$(find docs/superpowers/plans -maxdepth 1 -type f \( -name "*issue-$number-*.md" -o -name "*issue-$number.md" \))
    elif [[ -e docs/superpowers/plans ]]; then echo 'Outlines path is not a directory' >&2; exit 1; fi
    spec=$(jq -Rn --arg lines "$spec_paths" '$lines | split("\n") | map(select(length > 0)) | if length <= 1 then .[0] else error("ambiguous specs") end')
    plan=$(jq -Rn --arg lines "$plan_paths" '$lines | split("\n") | map(select(length > 0)) | if length <= 1 then .[0] else error("ambiguous outlines") end')
    progress=null
    if [[ "$plan" != null ]]; then
        progress=$(jq -Rs 'split("\n") | map(select(test("^\\s*- \\[[ xX]\\]"))) | {total:length,done:map(select(test("\\[[xX]\\]")))|length}' "$plan_paths")
    fi
    pr=null
    if [[ "$branch" != null ]]; then
        gh pr list --repo "$repo" --head "$(jq -r . <<< "$branch")" --state open --limit 2 --json number,url,state > "$work/prs.json"
        pr=$(jq -c 'if type == "array" and length <= 1 and all(.[]; (.number|type)=="number" and (.url|type)=="string" and .state=="OPEN") then .[0] else error("ambiguous/malformed open PRs") end' "$work/prs.json")
    fi
    jq -cn --argjson issue "$issue" --arg status "$status" --argjson blocked_by "$blockers" \
        --argjson branch "$branch" --argjson spec "$spec" --argjson plan "$plan" --argjson progress "$progress" --argjson pr "$pr" \
        '$issue | {number,title,url:.html_url,labels:[.labels[].name],status:$status,blocked_by:$blocked_by,
        local:{branch:$branch,spec:$spec,plan:$plan,plan_progress:$progress,open_pr:$pr}}' >> "$work/candidates.jsonl"
done < "$work/rows.jsonl"
jq -sn --argjson milestone "$milestone_number" --slurpfile candidates "$work/candidates.jsonl" --slurpfile skipped "$work/skipped.jsonl" \
    '{milestone:$milestone,candidates:($candidates|sort_by(.number)),skipped:($skipped|sort_by(.number))}'
