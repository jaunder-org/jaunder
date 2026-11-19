# Runtime ownership/partition checks replace the retired embedded Rust catalog.
# Every {backend, browser} pair has the same three-lane split: ordinary 1-of-2,
# ordinary 2-of-2 and one serial-special lane. Unsplit lanes are disabled controls.
def identity:
  "\(.backend)-\(.browser)-" +
  (if .partition == "ordinary" then "ordinary-\(.shardIndex)-of-\(.shardCount)" else .partition end);
# The seed repeats to fill the 32-hex-digit trace id (and 16-digit span id).
def trace_id: .traceSeed * (32 / (.traceSeed | length));
. as $catalog | .lanes as $lanes
| [$lanes[] | select(.enabled)] as $enabled
| [.experimentalProjectDependencies | keys[]] as $projects
| ([$enabled[] | {backend, browser}] | unique) as $pairs
| if .schemaVersion == 1 and ($enabled | length) == 12 and ($pairs | length) == 4
    and ([$lanes[].identity] | length == (unique | length))
    and ([$lanes[] | trace_id] | length == (unique | length))
    and all($lanes[]; .identity == identity
      and (.backend == "sqlite" or .backend == "postgres")
      and (.browser == "chromium" or .browser == "firefox")
      and (.traceSeed | test("^[0-9a-f]{1,2}$") and (test("^0+$") | not))
      and (.projects | length > 0 and length == (unique | length))
      and (if .partition == "ordinary" then .enabled
        and .shardCount == 2 and (.shardIndex == 1 or .shardIndex == 2)
        else .shardIndex == null and .shardCount == null
        and ((.partition == "unsplit" and (.enabled | not))
          or (.partition == "serial-special" and .enabled)) end))
    and all($pairs[]; . as $pair
      | [$enabled[] | select(.backend == $pair.backend and .browser == $pair.browser)] as $split
      | [$split[] | select(.partition == "ordinary")] as $ordinary
      | [$split[] | select(.partition == "serial-special")] as $special
      | ($split | length) == 3
        and ([$ordinary[].shardIndex] | sort) == [1, 2] and ($special | length) == 1
        and $ordinary[0].projects == $ordinary[1].projects
        and ([$split[].projects[]] | unique | sort)
          == ($projects | map(select(startswith("\($pair.browser)-"))) | sort)
        and all($ordinary[0].projects[]; . as $project | $special[0].projects | index($project) == null))
    and ([$enabled[].projects[]] | unique | sort) == ($projects | sort)
    and all($catalog.experimentalProjectDependencies | to_entries[]; . as $entry
      | (.value | length == (unique | length))
      and all($entry.value[]; . as $dependency
        | $dependency != $entry.key
        and all($enabled[] | select(.projects | index($entry.key) != null);
          (.projects | index($dependency)) != null
          and (.projects | index($dependency)) < (.projects | index($entry.key)))))
  then
    "lanes=\($enabled | map({backend, browser, lane: .identity}) | tojson)",
    "reconciles=\($pairs | tojson)"
  else error("invalid retained E2E ownership/partition catalog") end
