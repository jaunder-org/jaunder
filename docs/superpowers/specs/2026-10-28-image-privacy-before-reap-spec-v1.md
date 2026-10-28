# Minimal image-privacy `before_reap` coverage spec (#1841 follow-up)

**Decision:** propose one private unit test in
`host/src/image_privacy/test_control.rs`'s existing `#[cfg(test)] mod tests`; do
not use the consumer seam. This is the smallest deterministic seam and is a
test-only change. It intentionally proves the control's local contract, not that
a real child was reaped.

**Behavior and ordering:** construct `WorkerTestControl::new(mode)` directly,
then call its private `before_reap()` future and poll it once synchronously
using `std::future::Future::poll`, `std::pin::pin!` (or `Box::pin`), and
`Context` from a `Waker::noop()` (standard library; no new dependency, timer,
task, barrier, or rendezvous). For each non-Paused mode—`OversizedResponse`,
`ExpandedOutput`, `VerificationFailure`—assert the first poll is
`Poll::Ready(())`. Also poll `Paused` and assert `Poll::Pending`, establishing
the positive control. No await/sleep/time budget: this is an immediate readiness
assertion, not an eventual-completion race. Polling `Paused` registers its
existing `Notify` waiter; dropping that future is safe and does not release or
spawn anything.

**Red capability:** a mutant that unconditionally awaits the pause rendezvous
(or routes any non-Paused mode through that hook) makes at least one non-Paused
first poll `Pending`, failing the test. A mutant removing the Paused wait makes
the positive-control assertion fail. This directly exercises the private mode
predicate and actual hook future, rather than asserting an unrelated consumer
outcome. It will detect predicate/hook regressions, not errors in
`stop_and_wait` call ordering or process reaping.

**Scope/package/proof:** exact file above, `host` unit-test package; give the
test a unique filter such as
`before_reap_returns_immediately_for_non_paused_modes`. Focused proof:
`devtool run -- cargo xtask test-local -- -p host before_reap_returns_immediately_for_non_paused_modes`
(the supported focused form is documented in `CONTRIBUTING.md`; do not execute
in this intake). This proves only the targeted behavior. Authoritative coverage
remains the unfiltered Nix `coverage` producer over root-workspace nextest on
SQLite and PostgreSQL, followed by its coverage consumer; no filter, exemption,
suppression, or gate change is proposed.

**Why not consumer-only:** `process.rs::stop_and_wait` returns before
`before_reap` when `try_wait` says the child already exited. The existing
`server/tests/misc/media_handlers.rs::real_worker_response_bound_rejects_and_reaps_before_cleanup`
uses `OversizedResponse`, whose worker writes output and returns; response
completion/reaping therefore does not deterministically prove the hook ran. No
existing deterministic consumer rendezvous establishing the needed order was
found in the supplied review. A consumer test would need new synchronization or
make a timing assumption, contrary to the no-new-rendezvous constraint. The
direct test is meaningful for the stated hook contract but makes no
consumer-level causal claim.

**Evidence and limits:** refreshed checkout
`/home/mdorman/src/jaunder/herdsman-work-5d6bc07e`, branch
`herdsman/work-5d6bc07e`, HEAD `f0bd4c31883b3e6860db9659e662a9ca68502553`, tree
`06cbc2a8b1e5f64f859a5a976ab94045ead2b25c`, clean at intake. Source:
`host/src/image_privacy/test_control.rs:16,78-83,162-320`; call/early-return:
`host/src/image_privacy/process.rs:212-223`; consumer and package correction:
`server/tests/misc/media_handlers.rs:225-254`, `server/Cargo.toml` (`jaunder`,
integration target); command/coverage contract: `CONTRIBUTING.md:703-746`.
Retained report
`/tmp/pr1841-coverage/gcroots/coverage/diagnostics/text-report.log:27290-27298`
records three Paused calls and zero count at the `before_reap` closing line;
prior diagnosis `/tmp/pr-1841-ci-diagnosis.md` qualifies base coverage as passed
and source as byte-identical, not proof of dependency independence. Why the
aggregate gate failed and whether this gap triggered it remain unknown. No
commands/tests executed, source or tracker changed, or coverage campaign run;
only this `/tmp` proposal is produced.
