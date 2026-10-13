# Issue #1755: Local seed telemetry uses the owned Collector

## Outcome

The canonical fixture seed in `cargo xtask e2e-local` exports telemetry from
production `jaunder` and `test-support` children to the invocation-owned healthy
Collector. A local run needs no pre-existing receiver, and its retained capture
includes completed seed operations alongside existing browser/server evidence.

## Load-bearing decisions

- `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` is the explicit caller input to
  `devtool seed-e2e`; no endpoint flag or parallel configuration policy is
  added.
- The `devtool` command root resolves that input once before spawning seed
  children and explicitly injects the resolved endpoint into both child kinds,
  following ADR-0158. Child environment construction does not reread ambient
  configuration or mutate the parent process environment.
- The local host supplies its owned healthy Collector's gRPC exporter endpoint.
  An absent input preserves the standalone fallback `http://127.0.0.1:4317`. The
  VM continues supplying that same fixed endpoint explicitly.
- Canonical fixture contents and invocation order, both seed resource tags,
  `JAUNDER_CAPTURE_DIR` and capture layout remain unchanged. Browser
  attribution, request correlation, and existing capture/browser gates remain
  unchanged.
- Existing one-shot telemetry guard ownership and exit flushing remain intact.
  Seed spawn and command failures remain failures; ADR-0011 exporter diagnostics
  remain visible and nonfatal to command status. Export failures are not made
  fatal, swallowed, or reclassified by this endpoint-wiring fix.
- This is routine process wiring within existing contracts, requiring neither a
  new ADR nor an implementation outline.

## Acceptance

- Focused endpoint-selection tests prove that a nondefault caller-supplied local
  endpoint reaches both production `jaunder` and `test-support` children. They
  also prove absent-input fixed fallback and the explicit VM fixed endpoint.
- A real `devtool seed-e2e` child-process wiring test exercises command-root
  environment resolution and the environment delivered to both child kinds,
  using child-command environment configuration, not parent environment
  mutation. Use the smallest existing selection and command-consumer seams.
- After approval, run the focused local consumer with no pre-existing receiver:

  ```text
  devtool run -- env -u JAUNDER_TRACE_FILTER JAUNDER_LOG_FILTER=warn cargo xtask e2e-local auth.spec.ts
  ```

- The run succeeds with existing browser/capture checks intact and no
  connection-refused exporter warnings from seed processes.
- Its retained, completed `otel-traces.jsonl`, inspected after seed child exit
  and Collector finalization, contains resource attribute
  `jaunder.e2e.seed_process` values `e2e.seed.jaunder` and
  `e2e.seed.test-support`, each associated with real canonical seed storage
  work. Require `storage.site_config.set` for the production configuration
  writes and `storage.user.create_user.insert_user_row` for helper user
  creation; these are current real call-path spans, not synthetic verifier
  examples or a historical span-count expectation.
- Reuse `test-support verify-seed-trace` for its existing two-process storage
  evidence contract, supplemented by inspection of those actual operation spans.
  Resource markers alone are insufficient; no new validation framework or gate
  is required. Preserve existing VM endpoint and verification behavior.
- Execution logs and captures are ignored review evidence, not source
  deliverables. Report actual commands, outcomes, revision, and artifact paths.

## Boundaries

No dependency on #1742 or #1743; no VM changes or full VM campaign. No new
production API, authentication behavior, log/trace policy, capture schema,
fixture policy, telemetry lifecycle, or unrelated tooling cleanup. The change is
limited to endpoint handoff, focused regression coverage, and local proof.
