# ADR-0038: The Emacs AtomPub Transport Uses `plz` (curl), Not `url.el`

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-29

## Context and Problem Statement

C1 (#159) fills the `jaunder--http-request` seam — the Emacs client's
authenticated AtomPub HTTP transport. It was first built on Emacs' built-in
`url.el`: set
`url-request-method`/`url-request-extra-headers`/`url-request-data` (the Basic
app-password header among them), call `url-retrieve-synchronously`, then parse
the raw response buffer with a hand-written `jaunder--parse-http-response`. This
passed every local gate and the full host-side live suite.

In CI, the `e2e-elisp-integration` nixosTest then hit a **rare, intermittent
401**: an authenticated request that should have returned 200 came back
unauthorized, as if the `Authorization` header were absent. It did not reproduce
on demand.

Investigation isolated where the fault was **not**:

- **The server is clean.** 130/130 raw `curl` requests against the running
  server with the same credentials returned 200 — the server never spuriously
  401s.
- **`url.el` is clean _on the host_.** 100/100 paired probes (the `url.el`
  client vs a `curl` control, same endpoint and credentials) returned 200 on the
  development host — the header was never dropped there.
- **The flake is CI-VM-only.** The 401 appeared only inside the nixosTest VM,
  and only intermittently.

That triangulates the fault to `url.el`'s request-header handling under the CI
VM's timing/scheduling, not to the server, the credentials, or the test logic.
`url.el` carries request headers in the dynamically-bound special variable
`url-request-extra-headers`; that binding must still be live at the moment
`url.el` actually dispatches the connection, which under
`url-retrieve-synchronously` happens across process-filter and timer callbacks.
That is a narrow, timing-sensitive window — closed on the host (100/100), but
opening intermittently under the VM's different scheduling and producing an
authless request. A non-deterministic auth flake in the transport every other
unit (C/#74, D/#75) builds on is unacceptable: it would taint every downstream
e2e run with retryable-looking redness.

## Decision Drivers

- The transport must send the auth header **deterministically**, independent of
  host vs CI-VM timing.
- Eliminate the fragile dependency (`url.el` dynamic-variable header handling)
  rather than paper over the flake with retries.
- Keep the client dependency-light and the request construction
  explicit/inspectable.
- `curl` is already the trusted control in this very investigation (130/130) and
  is already present in the e2e/elisp VMs.

## Decision Outcome

**Build `jaunder--http-request` on `plz`, which drives the `curl` binary, and
drop `url.el` from the client entirely.**

- `jaunder--http-request` calls
  `(plz VERB url :headers … :body … :as 'response)`. Headers (the Basic
  app-password header from `jaunder--auth-secret`, plus `Content-Type` for
  writes) are passed as an explicit alist that `plz` renders to `curl --header`
  arguments. There is no dynamically-bound header variable and no callback
  window in which the binding can lapse — the header is in the `curl` argv or
  the request is never made.
- The hand-written raw-buffer parser is **removed**. `plz` already parses the
  response; `jaunder--plz-response->plist` converts a `plz-response` struct to
  the unchanged `(:status :headers :body)` plist (header names downcased for the
  existing case-insensitive `jaunder--response-header`). The plist contract
  C2–C4 consume is identical; only its producer changed.
- HTTP error statuses (4xx/5xx) are **returned in `:status`, not signalled**:
  `plz` raises `plz-error` on a non-2xx response, so the request is wrapped to
  catch `plz-error`, recover the carried `plz-response`, and return its plist; a
  transport-level failure with no response re-signals. A committed live test
  (`jaunder-transport-error-status-returned-not-signalled`) pins this 4xx
  behavior.
- **The live harness's auth-readiness gate also moves to `plz`.** The
  `jaunder-test--authed-200-p` poll that waits for the just-provisioned session
  to become usable previously used `url-retrieve-synchronously`; it now uses
  `plz`, so no part of the authenticated path rides `url.el` — exactly the
  transport this whole exercise removed.

`plz` is added to `emacsForCi` (`epkgs.plz`) and `curl` to the elisp/e2e VM and
the CI dev shell.

Rejected alternatives:

- **Retry the request on a 401 in `url.el`.** Masks a transport defect, can't
  distinguish a spurious 401 from a real one, and leaves the fragile dependency
  in every downstream unit.
- **Keep `url.el` and set the header another way** (e.g. a per-call `let`
  already in place). The header _was_ already bound at the call site; the
  failure is in `url.el`'s internal re-dispatch across callbacks, which the
  client cannot reach.
- **Shell out to `curl` directly.** Reinvents argv construction, response
  parsing, and error handling that `plz` already provides and that the Emacs
  ecosystem maintains; `plz` is the thin, well-scoped wrapper over exactly that.

## Consequences

- Good: the auth header is sent deterministically; the CI-VM 401 class is gone
  by construction, not by retry.
- Good: the response parser is deleted — `plz` owns parsing — shrinking the
  client's surface; the consumed `(:status :headers :body)` contract is
  unchanged so C2–C4 are unaffected.
- Good: `curl` (the investigation's trusted control, 130/130) is now the actual
  transport; behavior matches the control by construction.
- Neutral: a new elisp dependency (`plz`) and a runtime `curl` dependency. Both
  are small, ubiquitous, already vendored into the CI/VM toolchain, and keep the
  client far lighter than a bespoke HTTP stack.
- Bad: the client now requires a `curl` binary on `PATH` at runtime (a non-issue
  in the VMs and on any normal dev box, but a real deployment prerequisite to
  document for end users in a later unit).

## Verification

- Host live ERT suite green on `plz` (transport + 4xx error-path + #137 smoke).
- Host differential stress: 300/300 paired authenticated requests (the `plz`
  client vs a raw-`curl` control) all 200 — zero spurious 401s.
- CI-VM stress: the live integration suite looped 20× inside the
  `e2e-elisp-integration` nixosTest — the exact environment the original
  `url.el` flake required — all green.
