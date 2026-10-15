# cargo-mutants writes partial outcomes during execution. Only completed,
# applicable, mutually consistent evidence can authorize the counts below.
def require($condition; $message):
  if $condition then . else error($message) end;
def natural: type == "number" and . >= 0 and floor == .;
def nonempty: type == "string" and length > 0;
# Timestamp serializes in UTC, optionally with fractional seconds.
def timestamp: sub("\\.[0-9]+Z$"; "Z") | fromdateiso8601;
def failed:
  type == "object" and keys == ["Failure"] and
  (.Failure | type == "number" and floor == . and . != 0);
def phases($summary):
  .phase_results as $p |
  ($p | type == "array") and
  (all($p[]; (.duration | type == "number" and . >= 0) and
    (.argv | type == "array" and length > 0 and all(.[]; type == "string")))) and
  (if $summary == "Unviable" then
     ($p | length == 1) and $p[0].phase == "Build" and ($p[0].process_status | failed)
   else
     ($p | map(.phase)) == ["Build", "Test"] and
     $p[0].process_status == "Success" and
     (if $summary == "CaughtMutant" then $p[1].process_status | failed
      else $p[1].process_status == "Success" end)
   end);
def names($summary):
  [.outcomes[] | select(.summary == $summary) | .scenario.Mutant.name] | sort;
def lines: split("\n") | if last == "" then .[:-1] else . end | sort;

require(length == 1; "expected one outcomes document") | .[0] |
require(type == "object"; "invalid outcomes document") |
require((.end_time | nonempty) and (.start_time | nonempty);
  "missing completion evidence") |
require((.end_time | timestamp) >= (.start_time | timestamp);
  "completion precedes start") |
require((.outcomes | type == "array") and
  all([.total_mutants, .missed, .caught, .unviable, .timeout, .success][]; natural);
  "missing or malformed outcomes/counts") |
require(([.outcomes[] | select(.scenario == "Baseline")] | length) == 1;
  "expected one baseline") |
require(all(.outcomes[] | select(.scenario == "Baseline");
  .summary == "Success" and phases("Success")); "baseline did not complete successful tests") |
require(all(.outcomes[] | select(.scenario != "Baseline");
  (.scenario | type == "object" and keys == ["Mutant"]) and
  .scenario.Mutant.package == $package and .scenario.Mutant.file == $file and
  (.scenario.Mutant.name | nonempty) and
  (.summary == "MissedMutant" or .summary == "CaughtMutant" or .summary == "Unviable") and
  phases(.summary)); "inapplicable, incomplete or unsupported mutant outcome") |
[.outcomes[] | select(.scenario != "Baseline")] as $mutants |
require(.total_mutants == ($mutants | length) and
  ($mutants | map(.scenario.Mutant) | unique | length) == .total_mutants and
  .missed == (names("MissedMutant") | length) and
  .caught == (names("CaughtMutant") | length) and
  .unviable == (names("Unviable") | length) and .timeout == 0 and .success == 0;
  "inconsistent outcome records/counts or timeout holes") |
require(.total_mutants > 0; "no applicable mutants; target not verified") |
require((($status == 2) and .missed > 0) or (($status == 0) and .missed == 0);
  "producer exit contradicts outcomes") |
require(names("MissedMutant") == ($missed | lines) and
  names("CaughtMutant") == ($caught | lines) and
  names("Unviable") == ($unviable | lines) and ($timeout | lines) == [];
  "category lists contradict outcomes") |
"missed: \(.missed)\ncaught: \(.caught)\nunviable: \(.unviable)\ntimeout: \(.timeout)"
