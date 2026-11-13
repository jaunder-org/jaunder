# Human view of a host-proof JSON report: a diagnostic's rendered `report`
# (trace tables) verbatim, then one line per step. The producing executable's
# exit status remains the verdict.
(.report // empty),
(.steps[]
| "[\(if .skipped then "skip" elif .ok then " ok " else "FAIL" end)] \(.name)"
  + (if (.ok | not) or .duration_ms >= 1000 then " (\(.duration_ms) ms)" else "" end)
  + (if .detail then " — \(.detail)" else "" end))
