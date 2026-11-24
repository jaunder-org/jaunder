# Runtime ownership checks replace the retired embedded Rust catalog.
# Every {backend, browser} pair has three lanes, shards 1..3 of 3, each selecting
# the browser's `<browser>-visual` and `<browser>` projects; plain `--shard` then
# runs each test exactly once per pair. Those projects have no dependencies
# (Playwright's `--shard` would rerun a dependency in every shard). Unsharded
# lanes are disabled measurement controls selecting the same projects. Shards
# are sized by one per-browser Playwright shard-weight string
# (`PWTEST_SHARD_WEIGHTS`, e.g. "34:33:33"), one weight per shard.
def identity:
  "\(.backend)-\(.browser)-" +
  (if .shardIndex == null then "unsplit" else "\(.shardIndex)-of-\(.shardCount)" end);
def projects: ["\(.browser)-visual", .browser];
# The seed repeats to fill the 32-hex-digit trace id (and 16-digit span id).
def trace_id: .traceSeed * (32 / (.traceSeed | length));
.lanes as $lanes
| [$lanes[] | select(.enabled)] as $enabled
| ([$enabled[] | {backend, browser}] | unique) as $pairs
| if .schemaVersion == 1 and ($enabled | length) == 12 and ($pairs | length) == 4
    and (.shardWeights | keys) == ([$pairs[].browser] | unique)
    and all(.shardWeights[]; test("^[1-9][0-9]*(:[1-9][0-9]*){2}$"))
    and ([$lanes[].identity] | length == (unique | length))
    and ([$lanes[] | trace_id] | length == (unique | length))
    and all($lanes[]; .identity == identity
      and (.backend == "sqlite" or .backend == "postgres")
      and (.browser == "chromium" or .browser == "firefox")
      and (.traceSeed | test("^[0-9a-f]{1,2}$") and (test("^0+$") | not))
      and .projects == projects
      and (if .enabled then .shardCount == 3 and (.shardIndex | IN(1, 2, 3))
        else .shardIndex == null and .shardCount == null end))
    and all($pairs[]; . as $pair
      | [$enabled[] | select(.backend == $pair.backend and .browser == $pair.browser) | .shardIndex]
      | sort == [1, 2, 3])
  then
    "lanes=\($enabled | map({backend, browser, lane: .identity}) | tojson)",
    "reconciles=\($pairs | tojson)"
  else error("invalid retained E2E ownership catalog") end
