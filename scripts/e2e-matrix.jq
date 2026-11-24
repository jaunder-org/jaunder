# Runtime ownership checks replace the retired embedded Rust catalog.
# Each browser's shard count N is the number of weights in its Playwright
# shard-weight string (`PWTEST_SHARD_WEIGHTS`, e.g. "48:24:28" for N=3), so a
# browser's lanes and weights cannot disagree. Every {backend, browser} pair has
# N lanes, shards 1..N of N, each selecting the browser's one `<browser>`
# project; plain `--shard` then runs each test exactly once per pair. The
# project has no dependencies (Playwright's `--shard` would rerun a dependency
# in every shard). Unsharded lanes are disabled measurement controls selecting
# the same project.
def identity:
  "\(.backend)-\(.browser)-" +
  (if .shardIndex == null then "unsplit" else "\(.shardIndex)-of-\(.shardCount)" end);
# The seed repeats to fill the 32-hex-digit trace id (and 16-digit span id).
def trace_id: .traceSeed * (32 / (.traceSeed | length));
.lanes as $lanes
| .shardWeights as $weights
| [$lanes[] | select(.enabled)] as $enabled
| ([$enabled[] | {backend, browser}] | unique) as $pairs
| if .schemaVersion == 1 and ($pairs | length) == 4
    and ($weights | keys) == ([$pairs[].browser] | unique)
    and all($weights[]; test("^[1-9][0-9]*(:[1-9][0-9]*)+$"))
    and ([$lanes[].identity] | length == (unique | length))
    and ([$lanes[] | trace_id] | length == (unique | length))
    and all($lanes[]; .identity == identity
      and (.backend == "sqlite" or .backend == "postgres")
      and (.browser == "chromium" or .browser == "firefox")
      and (.traceSeed | test("^[0-9a-f]{1,2}$") and (test("^0+$") | not))
      and (if .enabled then .shardCount == ($weights[.browser] | split(":") | length)
        else .shardIndex == null and .shardCount == null end))
    and all($pairs[]; . as $pair
      | [$enabled[] | select(.backend == $pair.backend and .browser == $pair.browser) | .shardIndex]
      | sort == [range(1; ($weights[$pair.browser] | split(":") | length) + 1)])
  then
    "lanes=\($enabled | map({backend, browser, lane: .identity}) | tojson)",
    "reconciles=\($pairs | tojson)"
  else error("invalid retained E2E ownership catalog") end
