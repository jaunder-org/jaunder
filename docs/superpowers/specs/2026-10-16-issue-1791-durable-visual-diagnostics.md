# #1791: retained diagnostics for a future visual failure

## Authority and scope

The Human direction relayed by the established Speaker asks for maintained
additional diagnostics so a recurrence can be diagnosed. This extends the
approved Actions-edge investigation: diagnostics may be implemented without a
proven repair. The prior A/B/C/VM batch was inconclusive and its temporary
instrumentation was removed. No lasting new diagnostics had been delivered.

The Lead accepts failure-only observation at the existing visual matcher seam
and producer provenance at the existing host/Nix artifact boundary. A clean
checkout revision intentionally participates in the final VM check identity;
there is no promise of final-verdict reuse across distinct clean commits.
Product dependency derivations must not acquire that revision stamp. This is
producer attribution, not an inert salt or a new selector.

Root cause and visual repair remain unproved. This spec authorizes no product,
fixture-contract, baseline, tolerance, mask, assertion, retry or theme/CSR/root
behavior change. Normal review/hooks/CI and explicit per-PR merge approval
remain required. #1791 must not be described as causally resolved by diagnostic
proof.

## Maintained behavior

- Retain requesting checkout HEAD, complete committed tree, raw parent headers,
  ref/detached state and pre/post cleanliness using host Git. Freeze HEAD before
  reading its raw commit, including shallow-checkout parents. Dirty source is
  committed-base-only, not an attested working tree.
- Retain the selected Nix derivation/output independently before building, also
  on spawn/build/test failure. Join only a clean unchanged requesting revision
  with the matching guest input revision/lane read from that selected output.
  Missing/stale/mismatched/uncertain evidence is explicitly unattested.
  Provenance-only lift failures warn without changing the browser verdict.
  Acquisition failures retain bounded operation/error context and warn on
  stderr; missing selected markers remain distinct from unreadable or corrupt
  markers. Error context excludes child output, JSON payloads and private data.
- A reused immutable output does not prove current runtime execution or its
  original producing checkout. Preserve that distinction from new execution
  observations and allowlisted CI request annotations.
- Put the guest input sidecar outside the capture inventory, retaining it
  through existing artifact lift/upload paths. Preserve peer #1743's ordered
  shutdown, private exchange and complete capture/archive integrity semantics on
  future integration; do not copy its unrelated source or change its error
  ordering.
- Catch screenshot matcher rejection only. Keep signatures, callers,
  `document.fonts.ready`, screenshot options and original error object
  unchanged. Add no pass-path DOM/layout reads, observer, rAF, pointer movement
  or screenshot.
- On failure, attach bounded runtime and post-failure geometry, computed styles,
  font/loading and hover/focus/closed-state evidence. Label assertion-window and
  observation times; exact capture-time geometry, selected rendered face and
  initial pointer coordinates remain unknown when unmeasured.
- Bound diagnostic elapsed time within the remaining test budget; never extend
  that budget. Closed page, collection/attachment rejection or a pending promise
  cannot replace the original matcher error. Record ancillary unavailability.
- Retain original expected/actual/diff images, trace and matcher reporting
  through existing Playwright artifacts. A later observation is not a
  replacement image. Restrict fields to synthetic test styling/state and
  allowlisted provenance; exclude cookies, credentials, input values, full
  environment/DOM/text dumps.

## Source and acceptance work

Implementation starts from af972414366bea1417abffb3da3cc848d01f05b8, complete
tree 416c72f6814ad0af91cb7db2b08d8841988972e8. Main movement is recorded
separately; integrated-tree proof must be refreshed before delivery.

- [x] Final-source helper/type refresh: 11 isolated helper cases and TypeScript
      passed against the final browser-helper bytes, including runtime
      annotations. Failure-only visual helper and tests: pass inactivity,
      bounded context/attachment, identical original error on
      deadline/collection/attachment failure, explicit timing/font/privacy
      limits. This is not an application or VM proof.
- [x] Host provenance unit tests: frozen raw parents, shallow/detached/dirty,
      revision/selection mismatch, failed build selection retention, reused
      runtime unknown, previous valid marker/current missing or failed clearing.
- [x] Ancillary-error regressions: successful compatibility-lane artifact
      collection retains no verdict-bearing failures for marker
      removal/copy/permission errors; failed Git queries and corrupt/unreadable
      selected evidence retain contextual warnings and bounded payload-free
      acquisition errors. Narrow auxiliary Rust proof does not establish real
      failed-producer artifact retention.
- [x] Generated-script source-order regression and actual failed producer: guest
      marker before browser execution, diagnostics retained before the original
      failed assertion; no shutdown/archive changes.
- [x] Coordinated local negative control retained the original strict matcher
      mismatch, unchanged expected/actual/diff PNGs, trace, timed JSON and host
      provenance. Exact probe reversal followed by one normal strict theme test
      passed, including accessibility. This is not symptom reproduction, a
      repair or a baseline update.
- [x] Real SQLite/Chromium failed-build proof exposed a destructive second lift.
      The reviewed minimal correction preserves build-owned recovery; its native
      orchestration regression passed all four current-output/gcroot cases. One
      source-matched hermetic retest retained all eight available selected
      artifacts automatically in the final host tree, byte-identical, including
      report/capture/phase/lane manifest/guest marker. Original matcher failure
      remained primary; the temporary probe was exactly reversed.
- [x] Independent source Standards/Contract reviews, including the lift
      correction, accepted with no remaining findings.
- [ ] Lead acceptance of the independent retained-evidence check and complete
      final-tree conformance/integration review.
- [ ] Applicable source-matching normal hooks/current CI and explicit per-PR
      merge approval. Execution logs remain ignored.

Proof packets remain under `.xtask/issue-1791/`: `failure-proof/`,
`hermetic-failure-proof/`, `lift-fix/` and `hermetic-failure-retest/`. The first
hermetic packet remains a genuine retention failure, not relabeled green. The
retest's producer exited 1 on the deliberate mismatch; separate automatic
retention verification exited 0. Census and duration manifests not emitted in
that failed-prerequisite arm remain explicitly absent, not fabricated. Dirty
provenance is committed-base-only. Neither local/VM proof nor source review
establishes clean input attribution, current CI upload/delivery, visual
causality or issue resolution. No broad matrix or retry-until-fail campaign.
