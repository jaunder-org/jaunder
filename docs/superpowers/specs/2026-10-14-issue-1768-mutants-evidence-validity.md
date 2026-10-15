# Issue 1768: Honest mutation verification evidence

## Outcome

`.mutants-loop/verify.sh <package> <file>` reports authoritative mutation counts
only for a completed, applicable run. Invalid or incomplete execution fails
honestly instead of appearing to prove that the requested file is already fixed.

## Load-bearing decisions

- Validate producer outcome evidence and termination before authoritative counts
  or survivor conclusions: readable, well-formed outcomes; completed run;
  successful baseline; mutually consistent outcome records and counts; and
  examined mutants applicable to the requested package/file.
- Missing artifacts or required evidence never default to successful zero
  counts. Failed baseline, malformed/inconsistent evidence, interrupted
  execution, unsupported tool failures and timeout holes are failed
  verification.
- Zero applicable mutants is explicit non-verification with a nonzero exit, even
  when cargo-mutants itself reports success.
- Valid survivor findings remain normal results: retain counts, `STILL ALIVE`
  details and the caller's existing successful exit for a valid survivor run.
  Completed applicable runs with no survivors retain successful zero-missed
  output.
- Preserve the shared mutation invocation and its coverage/filter/isolation
  settings. Align local protocol guidance only where needed to explain actual
  producer statuses and the caller's evidence requirements.

## Acceptance

- Deterministic regressions execute the real `verify.sh` consumer through the
  existing auxiliary host-test lane, substituting only fake cargo command output
  and isolated producer-shaped artifacts; no expensive mutation matrix runs.
- Invalid-argument/no-artifact, missing/malformed/incomplete or inconsistent
  evidence, failed baseline, timeout and zero-applicable cases exit nonzero,
  explain non-verification, and never publish authoritative success counts.
- Completed applicable survivor and zero-survivor fixtures retain their normal
  counts, survivor details where applicable, and successful caller exits.

## Boundaries

No broad runner/CI redesign, new public interface/framework, mutation coverage
weakening, external skill edits, or historical incident attribution. No outline.
