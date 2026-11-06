# Runtime ownership/partition checks replace the retired embedded Rust catalog.
def identity:
  "\(.backend)-\(.browser)-" +
  (if .partition == "ordinary" then "ordinary-\(.shardIndex)-of-\(.shardCount)" else .partition end);
. as $catalog | .lanes as $lanes
| [$lanes[] | select(.enabled)] as $enabled
| [.experimentalProjectDependencies | keys[]] as $projects
| if .schemaVersion == 1 and ($enabled | length) == 8
    and ([$lanes[].identity] | length == (unique | length))
    and ([$lanes[].traceDigit] | length == (unique | length))
    and all($lanes[]; .identity == identity
      and (.backend == "sqlite" or .backend == "postgres")
      and (.traceDigit | test("^[0-9a-fA-F]$"))
      and (.projects | length > 0 and length == (unique | length))
      and (if .partition == "ordinary" then .enabled and .browser == "firefox"
        and .shardCount == 2 and (.shardIndex == 1 or .shardIndex == 2)
        else .shardIndex == null and .shardCount == null
        and ((.partition == "unsplit" and (.enabled == (.browser == "chromium")))
          or (.partition == "serial-special" and .enabled and .browser == "firefox")) end))
    and all(["sqlite", "postgres"][]; . as $backend
      | [$enabled[] | select(.backend == $backend and .browser == "chromium")] as $chromium
      | [$enabled[] | select(.backend == $backend and .browser == "firefox")] as $firefox
      | [$firefox[] | select(.partition == "ordinary")] as $ordinary
      | [$firefox[] | select(.partition == "serial-special")] as $special
      | ($chromium | length) == 1 and ($firefox | length) == 3
        and ([$ordinary[].shardIndex] | sort) == [1, 2] and ($special | length) == 1
        and $ordinary[0].projects == $ordinary[1].projects
        and ([$firefox[].projects[]] | unique | sort) == ($projects | sort)
        and all($ordinary[0].projects[]; . as $project | $special[0].projects | index($project) == null)
        and all($catalog.experimentalProjectDependencies | to_entries[]; . as $entry
          | (.value | length == (unique | length))
          and all($entry.value[]; . as $dependency
            | $dependency != $entry.key
            and all($firefox[] | select(.projects | index($entry.key) != null);
              (.projects | index($dependency)) != null
              and (.projects | index($dependency)) < (.projects | index($entry.key))))))
  then
    "chromium=\($enabled | map(select(.browser == "chromium") | {backend, lane: .identity}) | tojson)",
    "firefox=\($enabled | map(select(.browser == "firefox") | {backend, lane: .identity}) | tojson)",
    "backends=\($enabled | map(select(.browser == "firefox") | .backend) | unique | tojson)"
  else error("invalid retained E2E ownership/partition catalog") end
