//! The poll loop: snapshots in, one event log and one terminal report out.
//!
//! Generic over both the source and the clock, so the whole loop — strike budget,
//! rate-limit waiting, change detection, heartbeats, the 90-minute timeout — is
//! exercised offline and instantly.
//!
//! Note the return type: [`watch`] yields a `PrReport`, never a `Result`. Every
//! terminal state, *including* the tooling itself failing, is a report — so the one
//! outcome that most needs to be legible cannot become an error that never gets
//! written down.

use super::decide::{self, Phase, Progress, Step};
use super::gh::ApiError;
use super::snapshot::{CheckState, PrSnapshot, PrSource, RequiredChecks, RunRef};
use super::{Event, EventKind, Outcome, PrReport, Subject};

pub trait Clock {
    fn now_unix(&self) -> u64;
    fn now_rfc3339(&self) -> String;
    fn sleep_secs(&self, secs: u64);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
    fn now_rfc3339(&self) -> String {
        format_unix_utc(self.now_unix())
    }
    fn sleep_secs(&self, secs: u64) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

/// Unix seconds → RFC 3339 UTC, so the event log carries real timestamps without
/// pulling a date crate into a tool that needs nothing else from one.
fn format_unix_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm), shifted to a March-based year so
    // the leap day falls at the end and needs no special case.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

#[derive(Debug, Clone, Copy)]
pub struct WatchConfig {
    pub interval_secs: u64,
    pub timeout_mins: u64,
    pub once: bool,
    pub heartbeat_secs: u64,
    pub max_strikes: u32,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            interval_secs: 30,
            timeout_mins: 90,
            once: false,
            heartbeat_secs: 600,
            max_strikes: 5,
        }
    }
}

/// How a required context reads in the event log.
///
/// Not `{:?}` on the `Option<CheckState>`: a human watching stderr should see
/// `e2e gate: not yet reported`, not `e2e gate: None` — and "has not appeared yet" is
/// exactly the state the late-appearing aggregate check makes worth naming clearly.
fn check_state_label(snap: &PrSnapshot, name: &str) -> String {
    match decide::resolve_context(&snap.checks, name).map(|e| e.state) {
        None => "not yet reported".into(),
        Some(CheckState::Pending) => "pending".into(),
        Some(CheckState::Success) => "success".into(),
        Some(CheckState::Failure) => "failure".into(),
    }
}

/// The previous poll's emitted view, component by component.
///
/// This **is** the change-detection fingerprint, kept decomposed rather than hashed
/// into one string: emission is per changed component, so comparing component-wise is
/// what the event log actually needs. Note what it holds — phase, the required
/// contexts' states, queue membership and position, and the standing warning — and
/// what it deliberately omits: elapsed time, poll count, `updatedAt`, and any check
/// the ruleset does not require. Anything that ticks on its own would turn this from
/// a change-emitter into a per-poll emitter.
struct Rendered {
    phase: Phase,
    /// `(required context name, its resolved state)`, in **ruleset order**.
    ///
    /// Compared positionally against the previous poll's vector, which is sound only
    /// because the ruleset is fetched once and cached for the life of the watch. If
    /// that ever becomes a per-poll read, pair by name instead — otherwise a reordered
    /// or resized ruleset would silently mis-attribute states to contexts.
    checks: Vec<(String, String)>,
    queue: String,
    warn: Option<String>,
}

impl Rendered {
    fn of(snap: &PrSnapshot, req: &RequiredChecks, phase: Phase, warn: Option<String>) -> Self {
        Self {
            phase,
            checks: req
                .contexts
                .iter()
                .map(|name| (name.clone(), check_state_label(snap, name)))
                .collect(),
            queue: format!("{}:{:?}", snap.queue.in_queue, snap.queue.position),
            warn,
        }
    }
}

/// Everything the loop carries between polls, kept in one place so the emit logic can
/// borrow it without fighting the event sink.
struct Emitter<'a> {
    events: Vec<Event>,
    sink: &'a mut dyn FnMut(&Event),
    last_event_at: u64,
}

impl Emitter<'_> {
    fn emit(&mut self, at: String, now: u64, kind: EventKind, detail: String) {
        let event = Event { at, kind, detail };
        (self.sink)(&event);
        self.events.push(event);
        self.last_event_at = now;
    }
}

/// Poll `subject` until it reaches a terminal state, the budget expires, or the API
/// stops answering.
pub fn watch<S: PrSource, C: Clock>(
    source: &S,
    clock: &C,
    subject: &Subject,
    cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    let start = clock.now_unix();
    // Saturating: `--timeout` has no upper bound, and an absurd value should mean
    // "effectively forever", not a debug-build overflow panic.
    let deadline = start.saturating_add(cfg.timeout_mins.saturating_mul(60));
    let mut em = Emitter {
        events: Vec::new(),
        sink,
        last_event_at: start,
    };

    let mut required: Option<RequiredChecks> = None;
    let mut progress = Progress::default();
    let mut strikes = 0u32;
    let mut head_sha = String::new();
    let mut prev: Option<Rendered> = None;
    let mut ever_read = false;

    loop {
        let now = clock.now_unix();
        if now >= deadline {
            let at = clock.now_rfc3339();
            // Which terminal state this is turns on whether we ever managed to read
            // the PR at all. Riding out a rate limit that never clears reaches the
            // deadline having learned nothing — reporting that as `timed-out`
            // ("GitHub never finished") would send an agent looking at the queue when
            // the truth is we could not see. That conflation is the whole defect.
            let (outcome, detail) = if ever_read {
                (
                    Outcome::TimedOut,
                    "the watch budget expired; GitHub never finished",
                )
            } else {
                (
                    Outcome::WatcherError,
                    "the watch budget expired without a single successful read",
                )
            };
            em.emit(at, now, EventKind::Terminal, outcome.as_str().into());
            return finish(
                subject,
                head_sha,
                Terminal {
                    outcome,
                    detail: Some(detail.into()),
                    pointer: None,
                    phase: None,
                },
                em.events,
            );
        }

        // One fallible unit: the ruleset (fetched once), the snapshot, and — only in
        // the state where ejection is possible — the merge-group probe. A probe
        // failure is a poll failure, never a silent `None`, which would read as "not
        // ejected".
        let polled = (|| -> Result<(RequiredChecks, PrSnapshot, Option<RunRef>), ApiError> {
            let req = match &required {
                Some(r) => r.clone(),
                None => source.required_checks(subject)?,
            };
            let snap = source.snapshot(subject)?;
            let ejection = if decide::needs_ejection_probe(&snap, &req) {
                source.ejection_run(subject)?
            } else {
                None
            };
            Ok((req, snap, ejection))
        })();

        let (req, snap, ejection) = match polled {
            Ok(v) => {
                strikes = 0;
                ever_read = true;
                v
            }
            Err(e) => {
                let now = clock.now_unix();
                let at = clock.now_rfc3339();
                // Absorbed failures are still events. A silently swallowed error is
                // indistinguishable from "nothing changed" — the exact bug that made
                // the hand-rolled watchers look healthy while they were blind.
                em.emit(at, now, EventKind::PollError, e.detail());

                // Rate limiting is not a strike *when we know when it clears*: GitHub
                // says so, and waiting is strictly better than spending five strikes
                // over two minutes on a condition known to last twelve.
                if let ApiError::RateLimited {
                    reset_unix: Some(reset),
                } = e
                {
                    if reset >= deadline {
                        // Waiting would consume the whole budget and still not get an
                        // answer. Say so now instead of discovering it in 90 minutes.
                        return finish(
                            subject,
                            head_sha,
                            Terminal {
                                outcome: Outcome::WatcherError,
                                detail: Some(format!(
                                    "rate limited past the watch budget: {}",
                                    e.detail()
                                )),
                                pointer: None,
                                phase: None,
                            },
                            em.events,
                        );
                    }
                    if !cfg.once {
                        // A reset already in the past means the window has cleared;
                        // resume on the normal interval rather than spinning at one
                        // poll per second against a stale timestamp.
                        clock.sleep_secs(reset.saturating_sub(now).max(cfg.interval_secs));
                        continue;
                    }
                }

                // Everything else — including a rate limit whose reset we could not
                // learn (a secondary limit, or the `rate_limit` probe itself failing)
                // — goes through the strike budget. Treating an unattributed 403 as
                // terminal would end a 90-minute watch on one bad poll.
                strikes += 1;
                if !e.is_transient() && !matches!(e, ApiError::RateLimited { .. })
                    || strikes >= cfg.max_strikes
                    || cfg.once
                {
                    return finish(
                        subject,
                        head_sha,
                        Terminal {
                            outcome: Outcome::WatcherError,
                            detail: Some(format!(
                                "giving up after {strikes} failure(s): {}",
                                e.detail()
                            )),
                            pointer: None,
                            phase: None,
                        },
                        em.events,
                    );
                }
                clock.sleep_secs(cfg.interval_secs);
                continue;
            }
        };

        head_sha = snap.head_sha.clone();
        // An empty required set is silently permissive — nothing can ever be
        // `checks-failed` and the watch just runs to the budget. That happens for a
        // fork, a repo with no ruleset, or a token lacking the scope, all of which
        // return HTTP 200. Say so once rather than letting it look like patience.
        if required.is_none() && req.contexts.is_empty() {
            em.emit(
                clock.now_rfc3339(),
                now,
                EventKind::Warning,
                "the branch ruleset lists no required checks — nothing here can gate a merge"
                    .into(),
            );
        }
        required = Some(req.clone());

        let step = decide::classify(&snap, &req, ejection.as_ref(), &progress);
        let phase = match &step {
            Step::Continue { phase, .. } => *phase,
            Step::Terminal { .. } => Phase::Terminal,
        };

        // Emit per changed component, not per poll. The first poll emits the whole
        // current state so the log opens with where things stand.
        let warn = match &step {
            Step::Continue { warn, .. } => warn.clone(),
            Step::Terminal { .. } => None,
        };
        let current = Rendered::of(&snap, &req, phase, warn);
        let now = clock.now_unix();
        let at = clock.now_rfc3339();

        // On a terminal poll the component events would only restate what the
        // terminal event already says (a merged PR "leaving the queue", a check going
        // red that the outcome detail already names), so the outcome speaks alone.
        let terminal = matches!(step, Step::Terminal { .. });
        match &prev {
            _ if terminal => {}
            None => {
                em.emit(at.clone(), now, EventKind::Phase, phase.as_str().into());
                for (name, state) in &current.checks {
                    em.emit(
                        at.clone(),
                        now,
                        EventKind::Check,
                        format!("{name}: {state}"),
                    );
                }
                if snap.queue.in_queue {
                    em.emit(at.clone(), now, EventKind::Queue, queue_detail(&snap));
                }
            }
            Some(before) => {
                if before.phase != phase {
                    em.emit(at.clone(), now, EventKind::Phase, phase.as_str().into());
                }
                for ((name, state), (_, prev_state)) in current.checks.iter().zip(&before.checks) {
                    if state != prev_state {
                        em.emit(
                            at.clone(),
                            now,
                            EventKind::Check,
                            format!("{name}: {state}"),
                        );
                    }
                }
                if current.queue != before.queue {
                    em.emit(at.clone(), now, EventKind::Queue, queue_detail(&snap));
                }
            }
        }
        if let Some(text) = current.warn.as_ref().filter(|_| !terminal) {
            if prev.as_ref().and_then(|p| p.warn.as_ref()) != Some(text) {
                em.emit(at.clone(), now, EventKind::Warning, text.clone());
            }
        }
        prev = Some(current);

        // History the pure machine cannot derive: an entry that is gone now was only
        // ever visible as a transition.
        progress.was_queued |= snap.queue.in_queue;

        if let Step::Terminal {
            outcome,
            detail,
            pointer,
        } = step
        {
            em.emit(at, now, EventKind::Terminal, outcome.as_str().into());
            return finish(
                subject,
                head_sha,
                Terminal {
                    outcome,
                    detail,
                    pointer,
                    phase: None,
                },
                em.events,
            );
        }

        if cfg.once {
            return finish(
                subject,
                head_sha,
                Terminal {
                    outcome: Outcome::Pending,
                    detail: None,
                    pointer: None,
                    phase: Some(phase.as_str().into()),
                },
                em.events,
            );
        }

        // Silence must not read as either progress or death: if nothing has moved for
        // the heartbeat interval, say so.
        if clock.now_unix().saturating_sub(em.last_event_at) >= cfg.heartbeat_secs {
            let now = clock.now_unix();
            let at = clock.now_rfc3339();
            em.emit(
                at,
                now,
                EventKind::Heartbeat,
                format!(
                    "still {}, no change for {}m",
                    phase.as_str(),
                    cfg.heartbeat_secs / 60
                ),
            );
        }

        clock.sleep_secs(cfg.interval_secs);
    }
}

fn queue_detail(snap: &PrSnapshot) -> String {
    match snap.queue.position {
        Some(p) => format!("position {p}"),
        None => "queued".into(),
    }
}

/// How a watch ended, before it is joined with the subject and the event log.
struct Terminal {
    outcome: Outcome,
    detail: Option<String>,
    pointer: Option<String>,
    /// Only ever `Some` for `Pending`, which `--once` alone can produce.
    phase: Option<String>,
}

fn finish(subject: &Subject, head_sha: String, end: Terminal, events: Vec<Event>) -> PrReport {
    PrReport {
        outcome: end.outcome,
        pr: subject.number.0,
        head_sha,
        phase: end.phase,
        detail: end.detail,
        pointer: end.pointer,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::snapshot::CheckState;
    use crate::pr::test_support::*;

    #[test]
    fn unix_seconds_format_as_rfc3339_utc() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_utc(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(format_unix_utc(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(format_unix_utc(1_609_459_200), "2021-01-01T00:00:00Z");
    }

    // ---- sustained API failure is its own outcome ----

    #[test]
    fn five_consecutive_api_failures_yield_watcher_error() {
        let src = FakeSource::new(
            (0..5)
                .map(|_| Err(ApiError::Transport("boom".into())))
                .collect(),
            queue_rules(),
        );
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| {
            seen.push(e.clone())
        });

        // Must fail if the loop returns silence, success, or a check verdict.
        assert_eq!(
            report.outcome,
            Outcome::WatcherError,
            "sustained API failure must be watcher-error, not {:?}",
            report.outcome
        );
        assert_ne!(report.outcome, Outcome::Merged);
        assert_ne!(report.outcome, Outcome::ChecksFailed);
        assert_ne!(report.outcome, Outcome::TimedOut);
        assert!(
            !report.events.is_empty(),
            "silence must never be the answer"
        );
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::PollError)
                .count(),
            5,
            "every absorbed failure is an event"
        );
    }

    #[test]
    fn a_transient_failure_before_success_does_not_end_the_watch() {
        let src = FakeSource::new(
            vec![
                Err(ApiError::Transport("blip".into())),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    // ---- rate limiting is not a strike ----

    #[test]
    fn rate_limit_inside_the_budget_waits_and_continues() {
        let c = clock();
        let src = FakeSource::new(
            vec![
                Err(ApiError::RateLimited {
                    reset_unix: Some(600),
                }),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(c.now_unix() >= 600, "must have waited for the reset");
        assert!(report
            .events
            .iter()
            .any(|e| e.detail.contains("rate limited")));
    }

    #[test]
    fn rate_limit_beyond_the_budget_is_watcher_error_immediately() {
        let c = clock();
        let src = FakeSource::new(
            vec![Err(ApiError::RateLimited {
                reset_unix: Some(99_999),
            })],
            queue_rules(),
        );
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(c.now_unix() < 5_400, "must not burn the budget waiting");
    }

    // ---- emit on change only ----

    #[test]
    fn identical_consecutive_snapshots_emit_exactly_one_phase_event() {
        let src = FakeSource::new(
            vec![
                Ok(open_pending()),
                Ok(open_pending()),
                Ok(open_pending()),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let phase_events = report
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Phase && e.detail == "awaiting-checks")
            .count();
        assert_eq!(phase_events, 1, "unchanged state must not re-emit per poll");
    }

    #[test]
    fn a_queue_position_change_emits_per_change() {
        // Poll 1 emits the full current state (including the queue at position 3);
        // poll 2 emits only the changed component (position 2).
        let src = FakeSource::new(
            vec![Ok(queued_at(3)), Ok(queued_at(2)), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Queue)
                .count(),
            2
        );
    }

    #[test]
    fn a_non_required_check_changing_emits_nothing() {
        // The counterpart to the queue test above, through the real loop: change
        // detection must ignore checks the ruleset does not require, or every
        // unrelated lint job would produce an event.
        let mut noisy = queued_at(3);
        noisy.checks.push(check(
            "optional-lint",
            CheckState::Failure,
            "2026-07-30T14:05:00Z",
        ));
        let src = FakeSource::new(
            vec![Ok(queued_at(3)), Ok(noisy), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Check)
                .count(),
            2,
            "only the two required contexts, emitted once on the first poll"
        );
    }

    #[test]
    fn checks_render_as_words_not_debug_output() {
        // A human reads this stream live; `Some(Pending)` / `None` is not a report.
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let checks: Vec<&str> = report
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Check)
            .map(|e| e.detail.as_str())
            .collect();
        assert!(
            checks.iter().any(|d| d.ends_with(": pending")),
            "{checks:?}"
        );
        assert!(
            !checks
                .iter()
                .any(|d| d.contains("Some(") || d.contains("None")),
            "{checks:?}"
        );
    }

    #[test]
    fn an_ejected_pr_is_reported_through_the_loop_not_just_the_rule() {
        // The probe wiring — `needs_ejection_probe` → `ejection_run` → `classify` —
        // has to be exercised end-to-end, or a dropped result would pass every test
        // in `decide`. The PR is green, open, and unqueued, with a failed merge-group
        // run newer than its head: reachable with no prior sight of the queue entry.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection(Some(ejection("2026-07-30T14:30:00Z")));
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Ejected);
        assert!(report.pointer.unwrap().contains("/actions/runs/"));
    }

    #[test]
    fn a_probe_failure_is_a_poll_error_never_a_silent_not_ejected() {
        // Swallowing the probe error would read as "no ejection found", which is the
        // silent-failure shape this command exists to eliminate.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection_error(ApiError::Transport("probe down".into()));
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(report
            .events
            .iter()
            .any(|e| e.kind == EventKind::PollError && e.detail.contains("probe down")));
    }

    #[test]
    fn an_unattributed_rate_limit_is_absorbed_not_terminal() {
        // A secondary rate limit carries no reset (the `rate_limit` probe can fail
        // too). One such 403 must not end a 90-minute watch.
        let src = FakeSource::new(
            vec![
                Err(ApiError::RateLimited { reset_unix: None }),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    #[test]
    fn ten_minutes_of_stasis_emits_one_heartbeat() {
        // Poll-then-sleep, so poll k lands at t = 30*(k-1); poll 21 is exactly t=600.
        let mut snaps: Vec<_> = (0..21).map(|_| Ok(open_pending())).collect();
        snaps.push(Ok(merged_snapshot()));
        let src = FakeSource::new(snaps, queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Heartbeat)
                .count(),
            1
        );
    }

    // ---- budget & --once ----

    #[test]
    fn budget_expiry_is_timed_out_not_watcher_error() {
        // One scripted snapshot; the fake repeats it until the budget runs out.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::TimedOut);
        assert_ne!(
            report.outcome,
            Outcome::WatcherError,
            "the tooling worked fine"
        );
    }

    #[test]
    fn once_mode_returns_pending_without_looping() {
        let c = clock();
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let mut config = cfg();
        config.once = true;
        let report = watch(&src, &c, &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert_eq!(report.phase.as_deref(), Some("awaiting-checks"));
        assert_eq!(c.now_unix(), 0, "--once must not sleep");
    }

    #[test]
    fn once_mode_reaches_a_terminal_outcome_when_one_exists() {
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let mut config = cfg();
        config.once = true;
        assert_eq!(
            watch(&src, &clock(), &subject(), config, &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    #[test]
    fn every_event_reaches_both_the_sink_and_the_report() {
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(merged_snapshot())],
            queue_rules(),
        );
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| {
            seen.push(e.clone())
        });
        assert_eq!(seen, report.events, "one log, two renderings");
    }
}
