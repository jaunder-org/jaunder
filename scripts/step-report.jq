# Human view of a focused-proof JSON report (`test-local`, `e2e-local`): one
# line per step. The producing executable's exit status remains the verdict.
.steps[]
| "[\(if .skipped then "skip" elif .ok then " ok " else "FAIL" end)] \(.name)"
  + (if (.ok | not) or .duration_ms >= 1000 then " (\(.duration_ms) ms)" else "" end)
  + (if .detail then " — \(.detail)" else "" end)
