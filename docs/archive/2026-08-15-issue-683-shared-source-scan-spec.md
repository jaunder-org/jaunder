# #683 — shared xtask Rust-source scan

Issue: [#683](https://github.com/jaunder-org/jaunder/issues/683). Milestone:
Developer tooling & DX.

## Problem

Several xtask static checks independently discover Rust files, decode them as
UTF-8, and pass `(path, source)` pairs to a pure `problems()` analyzer. That
duplication creates divergent failure handling and makes a future scan check
another copy point. A source file that cannot be traversed or decoded must not
be omitted: omission removes that file from the gate's population without
reporting it.

The current tree already has `files::with_extension` for sorted recursive
discovery, but its read-and-run layer is duplicated across the simple checks,
the ident-gate runner, and target-architecture placement. `web_server_fns` is
not in that family: it carries source metadata, accumulates errors for multiple
consumers, and its registrar analyzer takes a separate file as input.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | Add a crate-private `steps::scan` module. It owns the common source-scan interface: discover every `.rs` file under the supplied roots through `files::with_extension`, lexically sort the combined file population, decode every discovered file with `read_to_string`, invoke the supplied analyzer on `(display_path, source)` pairs, and push exactly one named `StepResult`. It owns no check-specific policy.                       |
| **D2** | A missing root, filesystem traversal error, unreadable file, or invalid UTF-8 is a failing scan result with the root/path and I/O error in its detail. The scan never silently drops a policed file and never invokes an analyzer on a partial population. The combined source population is lexically sorted across every root; this makes target-architecture-placement's formerly filesystem-dependent diagnostic order deterministic. |
| **D3** | The helper has an injected reader beneath its public runner so unit tests prove both permission-style read failures and invalid-UTF-8 failures without altering production behavior. Existing pure `problems()` analyzers and their direct unit tests remain unchanged.                                                                                                                                                                   |
| **D4** | Migrate the matching one-input source scans: no-full-reload, proffered-filename, proffered-secret, SQLx-newtype-bind, test-pattern, target-architecture-placement, and the ident-gate runner used by rendered-HTML and its sibling gates. No migrated step retains a local read loop or Rust-file walker.                                                                                                                                 |
| **D5** | Do not route `web_server_fns` / server-fn-registrar or `sqlx_newtype_decode_check` through this helper. `WebSources` carries source metadata and accumulated read errors for multiple consumers. SQLx-newtype-decode builds a wider approve-set from declaration roots and a macro-crate model as well as policing `storage/src`; forcing either through a one-input `(path, source)` runner would discard needed behavior.               |
| **D6** | This is an internal xtask refactor. Gate rules, roots, each analyzer's diagnostic text for a given ordered readable population, command selection, and the `CommandResult` envelope remain unchanged. Target-architecture-placement's cross-file diagnostic order becomes deterministic. No ADR or `CONTEXT.md` change is needed.                                                                                                         |

## Acceptance criteria

- **AC1 — one source-scan module.** The migrated checks call the shared runner.
  They contain neither a local `rust_files` walker nor a local loop that reads
  Rust source files into `(String, String)` pairs.
- **AC2 — closed scan population.** A missing root, unreadable file, or invalid
  UTF-8 source makes the named step fail with a useful root/path error; no
  analyzer result can claim that a partial scan passed.
- **AC3 — preserved check semantics.** Given readable source trees, each
  migrated check passes every discovered source to its existing pure analyzer
  and preserves that analyzer's diagnostic text. The full combined population is
  lexically sorted for every migrated check; target-architecture-placement's
  formerly filesystem-dependent cross-file output is intentionally normalized.
- **AC4 — specialized scans remain specialized.** `web_server_fns` /
  server-fn-registrar and `sqlx_newtype_decode_check` retain their existing
  metadata/model-building and error-accumulation paths.
- **AC5 — regression proof.** Unit tests exercise sorted Rust discovery, a
  permission-style injected reader failure, and an actual invalid-UTF-8 fixture;
  existing direct analyzer tests remain green.
- **AC6 — repository verification.** `cargo xtask check` passes.

## Risks

- **Changing when a scan stops on I/O error.** The former copies do not all
  format errors identically. D2 intentionally standardizes the safety property —
  no partial population — while retaining each analyzer's readable-source
  behavior.
- **Over-generalizing structural consumers.** D5 keeps `web_server_fns` out of
  the helper because its richer result is real behavior, not duplicate plumbing.
