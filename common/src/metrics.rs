//! Cardinality-safe OpenTelemetry metric emitters, shared by `web`, `server`,
//! and the CLI. Instruments are built once from the global meter; when no
//! `MeterProvider` is installed (no OTLP endpoint, or any non-server process)
//! they are no-ops. Helper arguments are bounded enums, so a call site can
//! never emit an unbounded attribute. See the design spec / ADR-0011.

use std::sync::LazyLock;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, KeyValue};

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
enum_attr!(EmailKind { Verification => "verification", PasswordReset => "password_reset" });
enum_attr!(SendResult { Success => "success", Failure => "failure" });
enum_attr!(UploadOutcome { Stored => "stored", Deduplicated => "deduplicated", QuotaExceeded => "quota_exceeded", TooLarge => "too_large", Invalid => "invalid" });
enum_attr!(ServeResult { Ok => "ok", NotFound => "not_found", NotModified => "not_modified" });
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
    errors: Counter<u64>,
    email_sent: Counter<u64>,
    email_send_duration: Histogram<u64>,
    media_uploads: Counter<u64>,
    media_upload_bytes: Histogram<u64>,
    media_served: Counter<u64>,
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
        errors: m.u64_counter("jaunder.errors").build(),
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
        media_served: m.u64_counter("jaunder.media.served").build(),
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

pub fn error(kind: &'static str, class: &'static str) {
    M.errors.add(
        1,
        &[
            KeyValue::new("error.kind", kind),
            KeyValue::new("error.class", class),
        ],
    );
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

pub fn email_send_duration_ms(ms: u64) {
    M.email_send_duration.record(ms, &[]);
}

pub fn media_upload(outcome: UploadOutcome) {
    M.media_uploads.add(1, &kv("outcome", outcome.as_str()));
}

pub fn media_upload_bytes(bytes: u64) {
    M.media_upload_bytes.record(bytes, &[]);
}

pub fn media_served(result: ServeResult) {
    M.media_served.add(1, &kv("result", result.as_str()));
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
    use super::*;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    #[tokio::test]
    async fn login_records_outcome_attribute() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        global::set_meter_provider(provider.clone());

        login(LoginOutcome::InvalidCredentials);
        provider.force_flush().expect("flush");

        let metrics = exporter.get_finished_metrics().expect("metrics");
        let found = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .any(|metric| metric.name() == "jaunder.auth.logins");
        assert!(found, "jaunder.auth.logins not exported");
    }
}
