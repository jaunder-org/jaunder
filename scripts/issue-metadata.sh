#!/usr/bin/env bash
# Shared issue metadata/readback only; each Just frontdoor owns its native action.
set -euo pipefail
repo=jaunder-org/jaunder
project=PVT_kwDOECw7os4BblPP
priority_field=PVTSSF_lADOECw7os4BblPPzhWUx50
work=$(mktemp -d)
number=''
trap 'code=$?; if (( code != 0 )); then printf "Issue operation failed; inspect %s/issues/%s before retrying (no rollback).\n" "$repo" "$number" >&2; fi; rm -r -- "$work"' EXIT

# gh owns REST pagination. Reject malformed pages and repeated identities before
# callers select rows, so partial data can never become an empty/successful list.
issue_list() {
    gh api --paginate --slurp "$1" > "$work/pages.json"
    jq -ce 'if type == "array" and length > 0 and all(.[]; type == "array") then add else error("invalid REST pages") end
        | if all(.[]; (.number | type == "number" and . > 0 and . == floor))
          and (map(.number) | length == (unique | length)) then . else error("invalid/repeated issue identity") end' "$work/pages.json"
}

resolve_milestone() {
    issue_list "repos/$repo/milestones?state=open&per_page=100" > "$work/milestones.json"
    milestone_number=$(jq -er --arg wanted "$1" '
        if all(.[]; (.title | type == "string" and length > 0)) then . else error("invalid milestone") end
        | map(select((.number | tostring) == $wanted or .title == $wanted))
        | if length == 1 then .[0].number else error("milestone must match exactly one open title/number") end' "$work/milestones.json")
}

prepare_issue() {
    title=$1 issue_type=$2 milestone=$3 priority=$4 labels=$5 body_file=$6
    [[ "$title" =~ ^(feat|fix|docs|refactor|test|chore|build|ci|perf)(\([a-zA-Z0-9_-]+\))?:\ .+ ]] || { echo 'Expected Conventional Commit title' >&2; return 1; }
    [[ "$issue_type" == Task || "$issue_type" == Bug || "$issue_type" == Feature ]] || { echo 'Explicit type must be Task, Bug or Feature' >&2; return 1; }
    case "$priority" in
        P1) option=2d6fa1d1 ;; P2) option=be01f9da ;; P3) option=0bba09bc ;; P4) option=4b1f556c ;;
        *) echo 'Explicit Priority must be P1/P2/P3/P4' >&2; return 1 ;;
    esac
    resolve_milestone "$milestone"
    # jq reads the file as bytes decoded as UTF-8 text, preserving its trailing
    # newline; shell command substitution must not hold the body.
    jq -en --arg title "$title" --arg type "$issue_type" --argjson milestone "$milestone_number" \
        --arg labels "$labels" --rawfile body "$body_file" '
        ($labels | split(",")) as $labels
        | if ($labels | length > 0 and all(.[]; length > 0)) and ($labels | length == (unique | length))
          and ($body | test("\\S")) then {title:$title,type:$type,milestone:$milestone,labels:$labels,body:$body}
          else error("explicit nonempty labels and body required") end' > "$work/input.json"
}

validate_blockers() {
    for blocker in "$@"; do
        [[ "$blocker" =~ ^[1-9][0-9]*$ && "$blocker" != "$number" ]] || { echo 'Invalid/self blocker' >&2; return 1; }
    done
}

finish_issue() {
    number=$(jq -er '.number | if type == "number" and . > 0 and . == floor then . else error("missing issue number") end' "$work/mutated.json")
    url=$(jq -er --arg number "$number" '.html_url | select(. == ("https://github.com/jaunder-org/jaunder/issues/" + $number))' "$work/mutated.json")
    printf 'Verify issue #%s: %s\n' "$number" "$url" >&2
    gh project item-add 1 --owner jaunder-org --url "$url" --format json > "$work/item.json"
    item=$(jq -er '.id | select(type == "string" and length > 0)' "$work/item.json")
    gh project item-edit --project-id "$project" --id "$item" --field-id "$priority_field" \
        --single-select-option-id "$option" --format json > "$work/priority.json"
    jq -e --arg id "$item" '.id == $id' "$work/priority.json" > /dev/null
    # Explicit blockers are additive: editing does not erase pre-existing native
    # dependencies. Resolve their database IDs; never send issue numbers as IDs.
    for blocker in "$@"; do
        [[ "$blocker" =~ ^[1-9][0-9]*$ && "$blocker" != "$number" ]] || { echo 'Invalid/self blocker' >&2; return 1; }
        gh api "repos/$repo/issues/$blocker" > "$work/blocker.json"
        id=$(jq -er --argjson n "$blocker" 'select(.number == $n and .state == "open" and (has("pull_request") | not)) | .id | select(type == "number" and . > 0 and . == floor)' "$work/blocker.json")
        issue_list "repos/$repo/issues/$number/dependencies/blocked_by?per_page=100" > "$work/blockers.json"
        jq -e 'all(.[]; (.id | type == "number" and . > 0 and . == floor))' "$work/blockers.json" > /dev/null
        present=$(jq -r --argjson id "$id" 'any(.[]; .id == $id)' "$work/blockers.json")
        if [[ "$present" == false ]]; then
            gh api --method POST "repos/$repo/issues/$number/dependencies/blocked_by" -F "issue_id=$id" > "$work/dependency.json"
            jq -e '(.id | type == "number" and . > 0 and . == floor) and (.number | type == "number" and . > 0 and . == floor)' "$work/dependency.json" > /dev/null
        fi
        issue_list "repos/$repo/issues/$number/dependencies/blocked_by?per_page=100" > "$work/blockers.json"
        jq -e --argjson id "$id" 'any(.[]; .id == $id)' "$work/blockers.json" > /dev/null
    done
    gh api "repos/$repo/issues/$number" > "$work/readback.json"
    jq -e --slurpfile expected "$work/input.json" --argjson n "$number" '
        .number == $n and .title == $expected[0].title and .body == $expected[0].body
        and .type.name == $expected[0].type and .milestone.number == $expected[0].milestone
        and ([.labels[].name] | sort) == ($expected[0].labels | sort)
        or error("issue metadata/body readback mismatch (possible body mangling)")' "$work/readback.json" > /dev/null
    jq -j '.body' "$work/readback.json" > "$work/body-readback"
    cmp -- "$body_file" "$work/body-readback"
    gh api graphql -F "number=$number" -f 'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){issue(number:$number){projectItems(first:100){nodes{id project{id} priority:fieldValueByName(name:"Priority"){... on ProjectV2ItemFieldSingleSelectValue{name}}} pageInfo{hasNextPage}}}}}' > "$work/project.json"
    jq -ce --arg project "$project" --arg priority "$priority" --arg item "$item" '
        if (.errors // [] | length) == 0 and .data.repository.issue.projectItems.pageInfo.hasNextPage == false
        then .data.repository.issue.projectItems.nodes else error("incomplete project readback") end
        | if type == "array" and all(.[]; (.id | type == "string" and length > 0) and (.project.id | type == "string" and length > 0)) then . else error("malformed project item") end
        | map(select(.project.id == $project))
        | if length == 1 and .[0].id == $item and .[0].priority.name == $priority then .[0]
          else error("Backlog/Priority readback mismatch") end' "$work/project.json" > "$work/backlog.json"
    jq --slurpfile backlog "$work/backlog.json" '. + {backlog:$backlog[0]}' "$work/readback.json"
}
