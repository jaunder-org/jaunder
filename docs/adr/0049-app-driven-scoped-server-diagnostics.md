# ADR-0049: App-driven scoped server-diagnostics capture

- Status: accepted
- Date: 2026-07-04
- Issue: [#144](https://github.com/jaunder-org/jaunder/issues/144)

> **Update (#227):** the feature is now enabled by `JAUNDER_CAPTURE_DIR` (the
> server writes `diag.log` within it) rather than the standalone
> `JAUNDER_DIAG_LOG_FILE` env var this ADR describes. The scoped-diagnostics
> design below — the WARN+ JSONL layer, the panic hook, and the zero-panic-gate
> consumption — is otherwise unchanged. See the capture-dir contract ADR
> ([drafts/e2e-capture-dir-contract.md](drafts/e2e-capture-dir-contract.md)).

## Context

Diagnosing an e2e failure meant grovelling the full systemd journal inside the
VM: kernel boot spam plus every INFO request line bury the actual signal (the
panic or the error). [ADR-0032](../0032-e2e-zero-panic-gate.md) (the zero-panic
gate) already had to grep that whole journal for `panicked at`, and
[ADR-0037](../0037-e2e-failure-diagnostics-capture.md) established the
capture-before-fail / recover-from-`--keep-failed` machinery — but 0037 is "the
harness copies whatever the VM already produced." Nothing produced a small,
low-noise artifact scoped to the server's own errors and panics. (Surfaced
2026-06-28 diagnosing #138/#129, where `rg` over the journal drowned in kernel
boot noise.)

The `otel-traces.jsonl` + `analyze-otel-traces` pair already proved the value of
a **scoped structured artifact** the app emits and tooling consumes. Failure
logs deserve the same treatment.

## Decision

**The app emits its own purpose-built, scoped diagnostic stream; the e2e gate
prefers it.** Four decisions, extending 0032 and 0037 rather than replacing
them:

1. **App-driven capture (vs 0037's harness-copies model).** An e2e-only
   `JAUNDER_DIAG_LOG_FILE` env var (mirroring the `JAUNDER_MAIL_CAPTURE_FILE` /
   `JAUNDER_WEBSUB_CAPTURE_FILE` idiom) turns on two feeders in the server: a
   `tracing` `fmt().json()` layer gated to **WARN and above** by its **own
   per-layer filter**, and a **panic hook**. Both append to that file.
   Production leaves the var unset, so the feature is entirely inert there. The
   harness (0037) then copies this file out per combo alongside the journal — a
   new artifact in the existing copy-all set, not a new mechanism.

2. **The panic hook uses an independent `O_APPEND` handle and bypasses
   `tracing`.** Panics do not flow through the `tracing` layer (the default hook
   sends them to stderr → journald, which is exactly why the gate historically
   grepped the kernel-laden journal). The hook opens its **own** `File` in
   append mode and writes the record directly — it must **not** share the
   layer's writer or route through `tracing::error!`. If a thread panics while
   holding a shared writer's lock (or a subscriber-internal lock), re-acquiring
   it on the panicking thread would deadlock, turning a captured panic into a
   silent hang — the worst outcome for a diagnostics feature. `O_APPEND`
   positions each `write()` at EOF atomically, so the hook's records interleave
   with the layer's WARN+ lines at line boundaries without any shared lock. The
   hook chains to the previous hook, so stderr → journald still fires and the
   journal remains the fallback.

3. **JSONL format.** One record per line, so tooling can be built on it (as
   `analyze-otel-traces` is built on `otel-traces.jsonl`). Tracing events use
   `fmt().json()`; panic records are a distinct object carrying `kind: "panic"`,
   a `message` containing the literal `panicked at <location>` substring, and a
   `location` field equal to `Location::to_string()` verbatim.

4. **The zero-panic gate sources panics from the UNION of the scoped file and
   the journal (a change to 0032's mechanism).** The gate scans **both** for
   `panicked at` (raw substring, not JSON parsing — so a rare torn line can't
   crash it) and fails, default-deny, on a hit in **either**. Results are
   de-duped by **panic location** (the token after `panicked at `, trailing `:`
   stripped — canonical across both the JSON record and the default hook's
   journal line, since both derive from the same `Location`). The **scoped
   record wins** the report; a location seen only in the journal (a panic that
   fired before the hook was installed) is still reported. The existing
   `allowed_panics` allowlist is preserved.

## Consequences

- On an e2e failure, a small `jaunder-diag-<backend>.log` of only WARN+ events
  and panic records is captured and uploaded per combo — no kernel /
  INFO-request noise. "Look here first"; the full journal is demoted to
  last-resort fallback.
- The gate never loses a panic relative to the pre-#144 journal-only grep: the
  union keeps 0032's guarantee intact, and the scoped file adds precision (a
  `kind:"panic"` record vs. arbitrary log text mentioning "panicked at").
- The panic hook is installed only when `JAUNDER_DIAG_LOG_FILE` is set, so the
  production panic path is unchanged. The independent-handle / bypass-`tracing`
  rule is load-bearing and documented at the hook site — a future
  "simplification" that shares the writer would reintroduce the deadlock.
- The synchronous, unbuffered file sink (not `tracing_appender::non_blocking` /
  `BufWriter`) is deliberate: a `panic = abort` must not drop the very lines the
  feature exists to keep.
- Follow-ups: the growing set of capture-to-file env vars
  (`JAUNDER_MAIL_CAPTURE_FILE`, `JAUNDER_WEBSUB_CAPTURE_FILE`,
  `JAUNDER_DIAG_LOG_FILE`) should consolidate into a single output-dir contract
  (#227); a configurable diag level and a dedicated `analyze-*` tool over the
  diag JSONL are left open (the JSONL format keeps both cheap later).
- Ancestors extended: [ADR-0032](../0032-e2e-zero-panic-gate.md) (gate mechanism
  changed here) and [ADR-0037](../0037-e2e-failure-diagnostics-capture.md) (this
  adds one artifact to its copy-all set).
