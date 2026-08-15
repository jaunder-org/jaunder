//! Cardinality-safe OpenTelemetry metric emitters, shared by `web` (its native
//! `#[server]` bodies), `server`, `storage`, and the CLI. Instruments are built once from the global
//! meter; when no `MeterProvider` is installed (no OTLP endpoint, or any
//! non-server process) they are no-ops. Helper arguments are bounded enums, or a
//! `&'static str` drawn from a closed set the call site cannot widen —
//! `atompub_request`'s `op` comes from `atompub_op` in
//! `server/src/atompub/mod.rs`, a matched-route-plus-method lookup, not from an
//! enum. Either way no call site can attach caller-supplied text as a label.
//! Exporter setup lives in the binary (`server::observability`), not here.
//!
//! This facade lives in `host` — the native-only shared crate (ADR-0058) — so
//! `opentelemetry` is kept out of the wasm bundle by crate structure rather than
//! a feature gate (issue #345). See ADR-0011 (amended) and ADR-0058.

use std::sync::LazyLock;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{KeyValue, global};

macro_rules! enum_attr {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug)]
        pub enum $name { $($variant),+ }
        impl $name {
            fn as_str(self) -> &'static str { match self { $(Self::$variant => $s),+ } }
        }
    };
}

enum_attr!(LoginOutcome { Success => "success", InvalidCredentials => "invalid_credentials", InternalError => "internal_error" });
enum_attr!(SessionOutcome { Ok => "ok", InvalidToken => "invalid_token", SessionNotFound => "session_not_found", Internal => "internal" });
enum_attr!(RegistrationSource { Web => "web", Cli => "cli" });
enum_attr!(RegistrationPolicy { Open => "open", InviteOnly => "invite_only", Closed => "closed", CliBypass => "cli_bypass" });
enum_attr!(RegistrationResult { Ok => "ok", Rejected => "rejected" });
enum_attr!(InviteEvent { Created => "created", Redeemed => "redeemed" });
enum_attr!(PasswordResetEvent { Requested => "requested", Completed => "completed" });
enum_attr!(EmailKind { Verification => "verification", PasswordReset => "password_reset", Invite => "invite" });
enum_attr!(SendResult { Success => "success", Failure => "failure" });
enum_attr!(UploadOutcome { Stored => "stored", Deduplicated => "deduplicated", QuotaExceeded => "quota_exceeded", TooLarge => "too_large", Invalid => "invalid", Error => "error" });
enum_attr!(RegenResult { Ok => "ok", Error => "error" });
enum_attr!(PingOutcome { Success => "success", Failed => "failed", Exhausted => "exhausted", NoHub => "no_hub" });
enum_attr!(CacheResult { Hit => "hit", Miss => "miss" });
enum_attr!(BackupResult { Success => "success", Failure => "failure" });
enum_attr!(PostEvent { Created => "created", Updated => "updated", Published => "published", Deleted => "deleted" });
enum_attr!(AtompubResult { Ok => "ok", ClientError => "client_error", ServerError => "server_error" });

struct Instruments {
    logins: Counter<u64>,
    session_validations: Counter<u64>,
    registrations: Counter<u64>,
    invites: Counter<u64>,
    password_resets: Counter<u64>,
    email_sent: Counter<u64>,
    email_send_duration: Histogram<u64>,
    media_uploads: Counter<u64>,
    media_upload_bytes: Histogram<u64>,
    feed_regenerations: Counter<u64>,
    feed_regen_duration: Histogram<u64>,
    websub_pings: Counter<u64>,
    feed_cache: Counter<u64>,
    backup_runs: Counter<u64>,
    backup_duration: Histogram<u64>,
    backup_bytes: Histogram<u64>,
    backup_pruned: Counter<u64>,
    posts: Counter<u64>,
    atompub_requests: Counter<u64>,
}

static M: LazyLock<Instruments> = LazyLock::new(|| {
    let m = global::meter("jaunder");
    Instruments {
        logins: m.u64_counter("jaunder.auth.logins").build(),
        session_validations: m.u64_counter("jaunder.auth.session_validations").build(),
        registrations: m.u64_counter("jaunder.auth.registrations").build(),
        invites: m.u64_counter("jaunder.auth.invites").build(),
        password_resets: m.u64_counter("jaunder.auth.password_resets").build(),
        email_sent: m.u64_counter("jaunder.email.sent").build(),
        email_send_duration: m
            .u64_histogram("jaunder.email.send_duration")
            .with_unit("ms")
            .build(),
        media_uploads: m.u64_counter("jaunder.media.uploads").build(),
        media_upload_bytes: m
            .u64_histogram("jaunder.media.upload_bytes")
            .with_unit("By")
            .build(),
        feed_regenerations: m.u64_counter("jaunder.feed.regenerations").build(),
        feed_regen_duration: m
            .u64_histogram("jaunder.feed.regeneration_duration")
            .with_unit("ms")
            .build(),
        websub_pings: m.u64_counter("jaunder.feed.websub_pings").build(),
        feed_cache: m.u64_counter("jaunder.feed.cache").build(),
        backup_runs: m.u64_counter("jaunder.backup.runs").build(),
        backup_duration: m
            .u64_histogram("jaunder.backup.duration")
            .with_unit("ms")
            .build(),
        backup_bytes: m
            .u64_histogram("jaunder.backup.bytes")
            .with_unit("By")
            .build(),
        backup_pruned: m.u64_counter("jaunder.backup.pruned").build(),
        posts: m.u64_counter("jaunder.posts").build(),
        atompub_requests: m.u64_counter("jaunder.atompub.requests").build(),
    }
});

#[inline]
fn kv(key: &'static str, value: &'static str) -> [KeyValue; 1] {
    [KeyValue::new(key, value)]
}

pub fn login(outcome: LoginOutcome) {
    M.logins.add(1, &kv("outcome", outcome.as_str()));
}

pub fn session_validation(outcome: SessionOutcome) {
    M.session_validations
        .add(1, &kv("outcome", outcome.as_str()));
}

pub fn registration(
    source: RegistrationSource,
    policy: RegistrationPolicy,
    result: RegistrationResult,
) {
    M.registrations.add(
        1,
        &[
            KeyValue::new("source", source.as_str()),
            KeyValue::new("policy", policy.as_str()),
            KeyValue::new("result", result.as_str()),
        ],
    );
}

pub fn invite(event: InviteEvent) {
    M.invites.add(1, &kv("event", event.as_str()));
}

pub fn password_reset(event: PasswordResetEvent) {
    M.password_resets.add(1, &kv("event", event.as_str()));
}

pub fn email_sent(kind: EmailKind, result: SendResult) {
    M.email_sent.add(
        1,
        &[
            KeyValue::new("kind", kind.as_str()),
            KeyValue::new("result", result.as_str()),
        ],
    );
}

/// Records `jaunder.email.sent` for a completed send attempt, deriving the
/// `result` attribute from the send outcome. Keeps the success/failure branch
/// (and its coverage) here rather than at every call site.
pub fn email_send_result<T, E>(kind: EmailKind, result: &Result<T, E>) {
    let outcome = if result.is_ok() {
        SendResult::Success
    } else {
        SendResult::Failure
    };
    email_sent(kind, outcome);
}

pub fn email_send_duration_ms(ms: u64) {
    M.email_send_duration.record(ms, &[]);
}

pub fn media_upload(outcome: UploadOutcome) {
    M.media_uploads.add(1, &kv("outcome", outcome.as_str()));
}

pub fn media_upload_bytes(bytes: u64) {
    M.media_upload_bytes.record(bytes, &[]);
}

pub fn feed_regeneration(result: RegenResult) {
    M.feed_regenerations.add(1, &kv("result", result.as_str()));
}

pub fn feed_regen_duration_ms(ms: u64) {
    M.feed_regen_duration.record(ms, &[]);
}

pub fn websub_ping(outcome: PingOutcome) {
    M.websub_pings.add(1, &kv("outcome", outcome.as_str()));
}

pub fn feed_cache(result: CacheResult) {
    M.feed_cache.add(1, &kv("result", result.as_str()));
}

pub fn backup_run(result: BackupResult) {
    M.backup_runs.add(1, &kv("result", result.as_str()));
}

pub fn backup_duration_ms(ms: u64) {
    M.backup_duration.record(ms, &[]);
}

pub fn backup_bytes(bytes: u64) {
    M.backup_bytes.record(bytes, &[]);
}

pub fn backup_pruned(count: u64) {
    M.backup_pruned.add(count, &[]);
}

pub fn post(event: PostEvent) {
    M.posts.add(1, &kv("event", event.as_str()));
}

pub fn atompub_request(op: &'static str, result: AtompubResult) {
    M.atompub_requests.add(
        1,
        &[
            KeyValue::new("op", op),
            KeyValue::new("result", result.as_str()),
        ],
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    use super::*;

    /// Every instrument this module owns, paired with the call that emits it.
    ///
    /// The list is the contract with the dashboards and alerts that query these
    /// names, so it is written out literally rather than derived from
    /// [`Instruments`] — a test that asks the code what it emits agrees with the
    /// code by construction and can never catch a rename.
    const EXPECTED_INSTRUMENTS: &[&str] = &[
        "jaunder.auth.logins",
        "jaunder.auth.session_validations",
        "jaunder.auth.registrations",
        "jaunder.auth.invites",
        "jaunder.auth.password_resets",
        "jaunder.errors",
        "jaunder.email.sent",
        "jaunder.email.send_duration",
        "jaunder.media.uploads",
        "jaunder.media.upload_bytes",
        "jaunder.feed.regenerations",
        "jaunder.feed.regeneration_duration",
        "jaunder.feed.websub_pings",
        "jaunder.feed.cache",
        "jaunder.backup.runs",
        "jaunder.backup.duration",
        "jaunder.backup.bytes",
        "jaunder.backup.pruned",
        "jaunder.posts",
        "jaunder.atompub.requests",
    ];

    /// Calls every emitter once. Kept separate from the assertions so the list of
    /// calls reads as an inventory: if a new emitter is added here without a name
    /// in `EXPECTED_INSTRUMENTS`, the round-trip assertion below fails.
    fn emit_one_of_everything() {
        login(LoginOutcome::InvalidCredentials);
        session_validation(SessionOutcome::InvalidToken);
        registration(
            RegistrationSource::Web,
            RegistrationPolicy::InviteOnly,
            RegistrationResult::Rejected,
        );
        invite(InviteEvent::Redeemed);
        password_reset(PasswordResetEvent::Requested);
        crate::error::InternalError::storage(sqlx::Error::RowNotFound).emit_boundary_failure();
        email_sent(EmailKind::Verification, SendResult::Success);
        email_send_duration_ms(12);
        media_upload(UploadOutcome::Deduplicated);
        media_upload_bytes(4096);
        feed_regeneration(RegenResult::Ok);
        feed_regen_duration_ms(7);
        websub_ping(PingOutcome::NoHub);
        feed_cache(CacheResult::Hit);
        backup_run(BackupResult::Success);
        backup_duration_ms(900);
        backup_bytes(1024);
        backup_pruned(3);
        post(PostEvent::Published);
        atompub_request("POST /feed", AtompubResult::ClientError);
    }

    /// The attribute sets recorded on a named `u64` counter, one per data point.
    ///
    /// Attributes are what make a counter useful — `jaunder.email.sent` alone
    /// cannot tell you whether sending is failing — so a mutant that drops or
    /// mislabels one is invisible to a name-only assertion.
    fn counter_attributes(
        metrics: &[opentelemetry_sdk::metrics::data::ResourceMetrics],
        name: &str,
    ) -> Vec<BTreeSet<(String, String)>> {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

        metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .filter(|metric| metric.name() == name)
            .filter_map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => Some(sum),
                _ => None,
            })
            .flat_map(opentelemetry_sdk::metrics::data::Sum::data_points)
            .map(|point| {
                point
                    .attributes()
                    .map(|kv| (kv.key.as_str().to_owned(), kv.value.to_string()))
                    .collect()
            })
            .collect()
    }

    fn attrs(pairs: [(&str, &str); 2]) -> BTreeSet<(String, String)> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn attrs4(pairs: [(&str, &str); 4]) -> BTreeSet<(String, String)> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    /// One provider install per process, so every assertion that needs an
    /// exporter lives in this single test.
    ///
    /// `global::set_meter_provider` is process-global and install-once in effect.
    /// Under `cargo nextest` — what the repo's gate runs — each test is its own
    /// process and this is unremarkable; under plain `cargo test` a second
    /// installing test in the same process would race this one. Adding emitters
    /// here rather than adding a second test keeps that hazard at one.
    #[tokio::test]
    async fn every_emitter_exports_its_instrument() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        global::set_meter_provider(provider.clone());

        emit_one_of_everything();
        // Both branches of the send-result mapping. The kinds are chosen to be
        // ones `emit_one_of_everything` does not use, so each assertion below can
        // only be satisfied by `email_send_result` itself — it emits through
        // `email_sent`, which is already called directly with `verification`.
        email_send_result(EmailKind::Invite, &Ok::<(), ()>(()));
        email_send_result(EmailKind::PasswordReset, &Err::<(), ()>(()));
        provider.force_flush().expect("flush");

        let metrics = exporter.get_finished_metrics().expect("metrics");
        let instrument_names: BTreeSet<&str> = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .map(opentelemetry_sdk::metrics::data::Metric::name)
            .collect();

        // Named individually: "21 instruments exported" would pass with the wrong
        // 21, and the point of the assertion is *which* names reach a collector.
        for want in EXPECTED_INSTRUMENTS {
            assert!(
                instrument_names.contains(want),
                "{want} was never exported — an emitter is silently dead. \
                 exported: {instrument_names:?}"
            );
        }

        // The other direction: an instrument emitted but not listed above means
        // the inventory has drifted from the code, which is how a rename ships
        // unnoticed.
        let expected: BTreeSet<&str> = EXPECTED_INSTRUMENTS.iter().copied().collect();
        let unexpected: Vec<&&str> = instrument_names
            .iter()
            .filter(|name| !expected.contains(*name))
            .collect();
        assert!(
            unexpected.is_empty(),
            "instruments exported but not in EXPECTED_INSTRUMENTS: {unexpected:?}"
        );

        // `email_send_result` is the only branch in this module. If it inverted,
        // every failed send would be counted a success — a metric that lies in
        // exactly the situation you consult it. Assert the attributes it derives
        // actually reach the exporter, on both arms.
        let email = counter_attributes(&metrics, "jaunder.email.sent");
        assert!(
            email.contains(&attrs([("kind", "invite"), ("result", "success")])),
            "Ok did not record result=success; got {email:?}"
        );
        assert!(
            email.contains(&attrs([("kind", "password_reset"), ("result", "failure")])),
            "Err did not record result=failure; got {email:?}"
        );

        let errors = counter_attributes(&metrics, "jaunder.errors");
        assert_eq!(
            errors,
            vec![attrs4([
                ("error.kind", "storage"),
                ("error.class", "bug"),
                ("error.disposition", "boundary"),
                ("telemetry.origin", "server"),
            ])]
        );
        // `counter_attributes` reads counters only. Asking it about a histogram
        // yields nothing rather than panicking — worth pinning, because a silent
        // empty result is how the two assertions above would go vacuously true if
        // the instrument were ever changed to a histogram.
        assert!(
            counter_attributes(&metrics, "jaunder.email.send_duration").is_empty(),
            "counter_attributes should ignore histograms"
        );
    }

    /// The attribute vocabulary, pinned literally.
    ///
    /// These strings are the values dashboards and alert rules match on, so a
    /// silent rename is a production-visible break that nothing else here would
    /// catch: `as_str` is private, every caller passes it straight through to a
    /// `KeyValue`, and no other assertion looks at the value. ADR-0011 asks for
    /// exactly this ("exhaustive table tests so every attribute mapping is
    /// exercised regardless of which request paths a given integration test
    /// happens to hit").
    ///
    /// Exhaustive by construction: each row lists every variant of its enum, and
    /// the `match` in `enum_attr!` means adding a variant without extending the
    /// row here leaves the new variant unasserted — so extend the row when you
    /// add one.
    #[test]
    fn attribute_values_are_the_documented_vocabulary() {
        assert_eq!(LoginOutcome::Success.as_str(), "success");
        assert_eq!(
            LoginOutcome::InvalidCredentials.as_str(),
            "invalid_credentials"
        );
        assert_eq!(LoginOutcome::InternalError.as_str(), "internal_error");

        assert_eq!(SessionOutcome::Ok.as_str(), "ok");
        assert_eq!(SessionOutcome::InvalidToken.as_str(), "invalid_token");
        assert_eq!(
            SessionOutcome::SessionNotFound.as_str(),
            "session_not_found"
        );
        assert_eq!(SessionOutcome::Internal.as_str(), "internal");

        assert_eq!(RegistrationSource::Web.as_str(), "web");
        assert_eq!(RegistrationSource::Cli.as_str(), "cli");

        assert_eq!(RegistrationPolicy::Open.as_str(), "open");
        assert_eq!(RegistrationPolicy::InviteOnly.as_str(), "invite_only");
        assert_eq!(RegistrationPolicy::Closed.as_str(), "closed");
        assert_eq!(RegistrationPolicy::CliBypass.as_str(), "cli_bypass");

        assert_eq!(RegistrationResult::Ok.as_str(), "ok");
        assert_eq!(RegistrationResult::Rejected.as_str(), "rejected");

        assert_eq!(InviteEvent::Created.as_str(), "created");
        assert_eq!(InviteEvent::Redeemed.as_str(), "redeemed");

        assert_eq!(PasswordResetEvent::Requested.as_str(), "requested");
        assert_eq!(PasswordResetEvent::Completed.as_str(), "completed");

        assert_eq!(EmailKind::Verification.as_str(), "verification");
        assert_eq!(EmailKind::PasswordReset.as_str(), "password_reset");
        assert_eq!(EmailKind::Invite.as_str(), "invite");

        assert_eq!(SendResult::Success.as_str(), "success");
        assert_eq!(SendResult::Failure.as_str(), "failure");

        assert_eq!(UploadOutcome::Stored.as_str(), "stored");
        assert_eq!(UploadOutcome::Deduplicated.as_str(), "deduplicated");
        assert_eq!(UploadOutcome::QuotaExceeded.as_str(), "quota_exceeded");
        assert_eq!(UploadOutcome::TooLarge.as_str(), "too_large");
        assert_eq!(UploadOutcome::Invalid.as_str(), "invalid");
        assert_eq!(UploadOutcome::Error.as_str(), "error");

        assert_eq!(RegenResult::Ok.as_str(), "ok");
        assert_eq!(RegenResult::Error.as_str(), "error");

        assert_eq!(PingOutcome::Success.as_str(), "success");
        assert_eq!(PingOutcome::Failed.as_str(), "failed");
        assert_eq!(PingOutcome::Exhausted.as_str(), "exhausted");
        assert_eq!(PingOutcome::NoHub.as_str(), "no_hub");

        assert_eq!(CacheResult::Hit.as_str(), "hit");
        assert_eq!(CacheResult::Miss.as_str(), "miss");

        assert_eq!(BackupResult::Success.as_str(), "success");
        assert_eq!(BackupResult::Failure.as_str(), "failure");

        assert_eq!(PostEvent::Created.as_str(), "created");
        assert_eq!(PostEvent::Updated.as_str(), "updated");
        assert_eq!(PostEvent::Published.as_str(), "published");
        assert_eq!(PostEvent::Deleted.as_str(), "deleted");

        assert_eq!(AtompubResult::Ok.as_str(), "ok");
        assert_eq!(AtompubResult::ClientError.as_str(), "client_error");
        assert_eq!(AtompubResult::ServerError.as_str(), "server_error");
    }

    /// `kv` is the one shared shape-builder: every single-attribute emitter goes
    /// through it, so a mutant that drops the key or value silently strips the
    /// attribute from a dozen instruments at once.
    #[test]
    fn kv_pairs_the_key_with_the_value() {
        let [pair] = kv("outcome", "success");
        assert_eq!(pair.key.as_str(), "outcome");
        assert_eq!(pair.value.to_string(), "success");
    }
}
