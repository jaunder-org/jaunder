//! Helper functions for row type conversions and cryptographic operations.

use std::{fmt, io, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::role_instant::impl_role_instant;
use crate::{EmailVerified, OperatorStatus, PostTag, SessionRecord, UserRecord};
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::ids::{PostId, TagId, UserId};
use common::session_label::SessionLabel;
use common::tag::{Tag, TagLabel};
use common::time::UtcInstant;
use common::token::TokenHash;
use common::username::Username;
use host::password;
use host::stored_password_hash::StoredPasswordHash;

/// The `sessions.created_at` storage timestamp role, distinct from
/// `last_used_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub(crate) struct SessionCreatedAt(UtcInstant);
impl_role_instant!(SessionCreatedAt, UtcInstant);

/// The `sessions.last_used_at` storage timestamp role, distinct from
/// `created_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub(crate) struct SessionLastUsedAt(UtcInstant);
impl_role_instant!(SessionLastUsedAt, UtcInstant);

/// A session label retained exactly until the repair-on-read display policy.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredSessionLabel(String);

impl StoredSessionLabel {
    #[cfg(test)]
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Preserves an already-selected primary result while reporting a failed
/// secondary operation exactly once. Owning modules wrap this with private,
/// context-specific finish helpers so fault injection cannot affect production.
pub(crate) fn preserve_after_secondary<T, E>(
    primary: T,
    secondary: Result<(), E>,
    kind: host::error::ErrorKind,
    class: host::error::ErrorClass,
    context: &'static str,
) -> T
where
    E: std::error::Error + 'static,
{
    if let Err(error) = secondary {
        host::error::report_swallowed(
            kind,
            class,
            context,
            host::error::SwallowedSource::Error(&error),
        );
    }
    primary
}

// ---------------------------------------------------------------------------
// UserRecord helpers
// ---------------------------------------------------------------------------

/// Password-bearing authentication projections use these named fields to avoid
/// transposing the distinct `email_verified` and `is_operator` domain types.
pub(crate) struct UserRecordParts {
    pub(crate) user_id: UserId,
    pub(crate) username: Username,
    pub(crate) display_name: Option<DisplayName>,
    pub(crate) bio: Option<Bio>,
    pub(crate) created_at: UtcInstant,
    pub(crate) last_authenticated_at: Option<UtcInstant>,
    pub(crate) email: Option<Email>,
    pub(crate) email_verified: EmailVerified,
    pub(crate) is_operator: OperatorStatus,
}

pub(crate) fn build_user_record(
    UserRecordParts {
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
        is_operator,
    }: UserRecordParts,
) -> UserRecord {
    // The `username`, `display_name`, and `email` columns decode straight into
    // their domain newtypes via the sqlx bridge (#438), which validates through
    // `FromStr`, so a corrupt/migrated value is rejected as a column-decode error
    // before we ever get here — this build step is infallible.
    UserRecord {
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
        is_operator,
    }
}

// ---------------------------------------------------------------------------
// SessionRecord helpers
// ---------------------------------------------------------------------------

struct SessionRecordParts {
    token_hash: TokenHash,
    user_id: UserId,
    username: Username,
    label: SessionLabel,
    created_at: SessionCreatedAt,
    last_used_at: SessionLastUsedAt,
}

fn build_session_record(
    SessionRecordParts {
        token_hash,
        user_id,
        username,
        label,
        created_at,
        last_used_at,
    }: SessionRecordParts,
) -> SessionRecord {
    // Every column arrives as its domain type — `token_hash`/`username` through the
    // validating string bridge (#438), `user_id` through the id bridge (#686), and
    // the timestamp pair through distinct role wrappers (#751) — so a
    // corrupt/migrated value is rejected as a column-decode error before we ever
    // get here, and adjacent timestamp swaps fail at the row-to-parts seam.
    SessionRecord {
        token_hash,
        user_id,
        username,
        label,
        created_at: created_at.value(),
        last_used_at: last_used_at.value(),
    }
}

// ---------------------------------------------------------------------------
// Post tag JSON helper
// ---------------------------------------------------------------------------

/// One validated JSON aggregate of tags before it is attached to a post identity.
#[derive(Debug, macros::SqlxBridge)]
#[sqlx_bridge(text)]
pub(crate) struct SerializedPostTags(ParsedPostTags);

/// Parsed aggregate payload retained once from the `SQLx` decode boundary.
#[derive(Debug)]
struct ParsedPostTags(Vec<PostTagJson>);

impl fmt::Display for ParsedPostTags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string(&self.0).map_err(|_| fmt::Error)?;
        formatter.write_str(&json)
    }
}

impl FromStr for SerializedPostTags {
    type Err = serde_json::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(ParsedPostTags(serde_json::from_str(value)?)))
    }
}

impl SerializedPostTags {
    pub(crate) fn into_tags(self, post_id: PostId) -> Vec<PostTag> {
        self.0
            .0
            .into_iter()
            .map(|r| PostTag {
                post_id,
                tag_id: r.tag_id,
                tag_slug: r.tag_slug,
                tag_display: r.tag_display,
            })
            .collect()
    }
}

/// Row shape for the JSON-aggregated tags column. Field names match the SQL
/// `json_object` keys verbatim, hence the matching `tag_` prefixes.
// Fields mirror the SQL `json_object` aggregation keys (tag_id/tag_slug/tag_display)
// this struct deserializes; renaming would need per-field `#[serde(rename)]` for no gain.
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#[expect(clippy::struct_field_names)]
#[derive(Debug, Deserialize, Serialize)]
struct PostTagJson {
    tag_id: TagId,
    tag_slug: Tag,
    tag_display: TagLabel,
}

#[derive(sqlx::FromRow)]
pub struct SessionRow {
    token_hash: TokenHash,
    user_id: UserId,
    username: Username,
    label: StoredSessionLabel,
    created_at: SessionCreatedAt,
    last_used_at: SessionLastUsedAt,
}
impl SessionRow {
    #[must_use]
    pub(crate) fn last_used_at(&self) -> UtcInstant {
        self.last_used_at.value()
    }
}

pub(crate) fn session_record_from_row(row: SessionRow) -> SessionRecord {
    // The `label` column decodes into a lossless storage role and is sanitized into a
    // `SessionLabel` via the lossy constructor rather than a validating decode: a
    // label is a best-effort *display* value, so a pre-existing out-of-range row
    // (empty, over-long) is repaired on read instead of failing the whole
    // `list_sessions` query.
    build_session_record(SessionRecordParts {
        token_hash: row.token_hash,
        user_id: row.user_id,
        username: row.username,
        label: SessionLabel::from_lossy(row.label.as_str()),
        created_at: row.created_at,
        last_used_at: row.last_used_at,
    })
}

pub(crate) type InviteTokenStateRow = (Option<UtcInstant>, UtcInstant);

pub(crate) fn classify_invite_token_state(
    row: Option<InviteTokenStateRow>,
    now: UtcInstant,
) -> TokenState {
    match row {
        None => TokenState::Missing,
        Some((Some(_), _)) => TokenState::AlreadyUsed,
        Some((None, expires_at)) if expires_at <= now => TokenState::Expired,
        Some((None, _)) => TokenState::Claimable,
    }
}

// ---------------------------------------------------------------------------
// Claim verification error helpers
// ---------------------------------------------------------------------------

pub(crate) type TokenStateRow = (Option<UtcInstant>, UtcInstant);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenState {
    Missing,
    AlreadyUsed,
    Expired,
    Claimable,
}

pub(crate) fn classify_token_state(row: Option<TokenStateRow>, now: UtcInstant) -> TokenState {
    match row {
        None => TokenState::Missing,
        Some((Some(_), _)) => TokenState::AlreadyUsed,
        Some((None, expires_at)) if expires_at <= now => TokenState::Expired,
        Some((None, _)) => TokenState::Claimable,
    }
}

pub(crate) fn email_verification_claim_error(
    row: Option<TokenStateRow>,
    now: UtcInstant,
) -> crate::UseEmailVerificationError {
    match classify_token_state(row, now) {
        TokenState::Missing => crate::UseEmailVerificationError::NotFound,
        TokenState::AlreadyUsed => crate::UseEmailVerificationError::AlreadyUsed,
        TokenState::Expired | TokenState::Claimable => crate::UseEmailVerificationError::Expired,
    }
}

pub(crate) fn password_reset_claim_error(
    row: Option<TokenStateRow>,
    now: UtcInstant,
) -> crate::UsePasswordResetError {
    match classify_token_state(row, now) {
        TokenState::Missing => crate::UsePasswordResetError::NotFound,
        TokenState::AlreadyUsed => crate::UsePasswordResetError::AlreadyUsed,
        TokenState::Expired | TokenState::Claimable => crate::UsePasswordResetError::Expired,
    }
}

// ---------------------------------------------------------------------------
// Cryptographic operations
// ---------------------------------------------------------------------------

/// Hashes `password`, returning the [`StoredPasswordHash`] that goes in the column.
///
/// Returns the newtype rather than a `String` because ADR-0063 §5 requires an existing
/// newtype on **every** surface carrying its value — read *and* write. Producing a bare
/// `String` here would leave callers re-deriving the type at the assignment, which §5
/// names explicitly as the pattern to avoid.
#[tracing::instrument(name = "crypto.password.hash", skip(password))]
pub(crate) async fn hash_password(password: password::Password) -> io::Result<StoredPasswordHash> {
    let operation = hash_password_operation(&password);

    hash_password_with(password, operation).await
}

pub(crate) type HashPasswordOperation =
    fn(&password::Password) -> Result<String, password::PasswordError>;

pub(crate) fn hash_password_operation(password: &password::Password) -> HashPasswordOperation {
    #[cfg(any(test, feature = "test-utils"))]
    if password.as_ref() == "force-hash-error-for-test-coverage" {
        return forced_hash_failure;
    }
    #[cfg(not(any(test, feature = "test-utils")))]
    let _ = password;

    password::hash
}

pub(crate) async fn hash_password_with(
    password: password::Password,
    operation: HashPasswordOperation,
) -> io::Result<StoredPasswordHash> {
    use std::str::FromStr;

    let hashed = tokio::task::spawn_blocking(move || operation(&password))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;

    StoredPasswordHash::from_str(&hashed).map_err(io::Error::other)
}

#[cfg(any(test, feature = "test-utils"))]
pub(crate) fn forced_hash_failure(
    password: &password::Password,
) -> Result<String, password::PasswordError> {
    match password::verify(password, NON_PASSWORD_ARGON2_FAILURE_HASH) {
        Err(password::PasswordError::VerificationFailed(source)) => {
            Err(password::PasswordError::HashingFailed(source))
        }
        _ => unreachable!("the fixed test hash always produces a non-password Argon2 failure"),
    }
}

/// Throwaway password hashed once to seed [`dummy_password_hash`].
const DUMMY_PASSWORD: &str = "jaunder-timing-equalization-dummy";

/// Valid Argon2id hash (default parameters) used only if runtime hashing of
/// [`DUMMY_PASSWORD`] ever fails, so initialization stays infallible (no
/// `unwrap`/`expect` in production). Regenerate with the same parameters as
/// `password::hash` if the Argon2 defaults change.
const DUMMY_PASSWORD_HASH_FALLBACK: &str = "$argon2id$v=19$m=19456,t=2,p=1$MlXSqqFgPKBHXn92Klja9Q$FCo2fJCKGcEhWHiq+R7lVdfcP/TpFgrVKfK6bMoB3CM";

/// Returns a fixed, valid Argon2id hash used to equalize authentication timing
/// on the absent-user path, mitigating username enumeration via timing (see
/// analysis §2.1).
///
/// `authenticate` runs a full Argon2 verification only when the username
/// exists; an attacker can otherwise distinguish "no such user" (fast) from
/// "wrong password" (slow). The absent path verifies against this hash so both
/// outcomes take the same time. It is computed once with the same default
/// Argon2 parameters as [`password::hash`], so the dummy verification
/// costs the same as a genuine one.
pub(crate) fn dummy_password_hash() -> &'static StoredPasswordHash {
    use std::sync::OnceLock;

    static HASH: OnceLock<StoredPasswordHash> = OnceLock::new();
    dummy_password_hash_with(&HASH, password::hash)
}

fn dummy_password_hash_with(
    hash: &std::sync::OnceLock<StoredPasswordHash>,
    operation: HashPasswordOperation,
) -> &StoredPasswordHash {
    use password::Password;
    use std::str::FromStr;

    hash.get_or_init(|| {
        let Ok(password) = Password::from_str(DUMMY_PASSWORD) else {
            unreachable!("the checked-in dummy password is valid")
        };
        let generated = match operation(&password) {
            Ok(generated) => generated,
            Err(error) => {
                report_dummy_password_hash_failure(&error);
                return fallback_dummy_password_hash();
            }
        };
        match StoredPasswordHash::from_str(&generated) {
            Ok(hash) => hash,
            Err(_) => unreachable!("password::hash returns a valid stored password hash"),
        }
    })
}

fn report_dummy_password_hash_failure(error: &(dyn std::error::Error + 'static)) {
    host::error::report_swallowed(
        host::error::ErrorKind::Internal,
        host::error::ErrorClass::Bug,
        "storage.auth.dummy_password_hash",
        host::error::SwallowedSource::Error(error),
    );
}

/// [`DUMMY_PASSWORD_HASH_FALLBACK`] as a [`StoredPasswordHash`].
///
/// A named function rather than an inline closure so the fallback is reachable from a
/// test: it is otherwise taken only when Argon2 hashing itself fails, which no test can
/// induce without breaking password hashing globally. Extracting it keeps the branch
/// covered honestly instead of `cov:ignore`-ing a permanent blind spot — and gives the
/// constant the validity test it never had.
fn fallback_dummy_password_hash() -> StoredPasswordHash {
    use std::str::FromStr;

    // A `match`, not `.unwrap_or_else(|_| unreachable!(…))`: on one line the
    // `unwrap_or_else` call is *covered* even though its closure never runs, which trips
    // the A1-guard (a covered line inside an `unreachable!` span means the exemption's
    // premise is falsified). Here only the `Err` arm carries the assertion, and it stays
    // genuinely unexecuted.
    match StoredPasswordHash::from_str(DUMMY_PASSWORD_HASH_FALLBACK) {
        Ok(hash) => hash,
        Err(_) => unreachable!("the fallback hash constant is non-empty"),
    }
}

pub(crate) type VerifyPasswordOperation =
    fn(&password::Password, &str) -> Result<bool, password::PasswordError>;

#[tracing::instrument(name = "crypto.password.verify", skip(password, hash, operation))]
pub(crate) async fn verify_password_with(
    password: password::Password,
    hash: StoredPasswordHash,
    operation: VerifyPasswordOperation,
) -> io::Result<bool> {
    // `as_ref` is the only door out of the secret surface; argon2 owns the verdict on
    // whether the stored string is a hash it can parse.
    tokio::task::spawn_blocking(move || operation(&password, hash.as_ref()))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

#[cfg(any(test, feature = "test-utils"))]
const NON_PASSWORD_ARGON2_FAILURE_HASH: &str =
    "$argon2id$v=1$m=65536,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[cfg(test)]
pub(crate) fn forced_verify_failure(
    password: &password::Password,
    _: &str,
) -> Result<bool, password::PasswordError> {
    password::verify(password, NON_PASSWORD_ARGON2_FAILURE_HASH)
}

#[cfg(test)]
pub(crate) mod swallowed_test {
    use std::future::Future;

    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    pub(crate) fn capture<T>(operation: impl FnOnce() -> T) -> (T, String) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        let value = tracing::subscriber::with_default(subscriber, operation);
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 trace");
        (value, text)
    }

    pub(crate) async fn capture_async<T>(operation: impl Future<Output = T>) -> (T, String) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let value = operation.await;
        drop(guard);
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 trace");
        (value, text)
    }

    pub(crate) fn assert_one_report(text: &str, context: &str) {
        assert_eq!(
            text.matches(r#""error.disposition":"swallowed""#).count(),
            1,
            "trace: {text}"
        );
        assert!(
            text.contains(&format!(r#""error.context":"{context}""#)),
            "trace: {text}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::QueryStorageExt;
    use crate::test_support::{Backend, backends};

    use common::test_support::{
        parse_bio, parse_display_name, parse_email, parse_session_label, parse_token_hash,
        parse_username,
    };
    use common::time::UtcInstant;
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn utc_instant_round_trips_directly_through_sqlx(#[case] backend: Backend) {
        let env = backend.setup().await;
        let expected = "2026-08-26T12:34:56.123456Z".parse::<UtcInstant>().unwrap();
        let actual = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, UtcInstant>("SELECT $1")
                .bind_storage(expected)
                .fetch_one(pool)
                .await
                .unwrap()
        });
        assert_eq!(actual, expected);

        let absent = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, Option<UtcInstant>>("SELECT $1")
                .bind_storage(None::<UtcInstant>)
                .fetch_one(pool)
                .await
                .unwrap()
        });
        assert_eq!(absent, None);
    }

    #[test]
    fn test_build_user_record() {
        let now = UtcInstant::now();
        let parts = UserRecordParts {
            user_id: UserId::from(1),
            username: parse_username("alice"),
            display_name: Some(parse_display_name("Alice")),
            bio: Some(parse_bio("Bio")),
            created_at: now,
            last_authenticated_at: Some(now),
            email: Some(parse_email("alice@example.com")),
            email_verified: EmailVerified::VERIFIED,
            is_operator: OperatorStatus::STANDARD,
        };
        let record = build_user_record(parts);
        assert_eq!(record.user_id, UserId::from(1));
        assert_eq!(record.username, "alice");
        assert_eq!(record.email.unwrap(), "alice@example.com");
    }

    #[test]
    fn test_build_session_record() {
        let now = UtcInstant::now();
        let later = UtcInstant::from(now.value() + chrono::Duration::seconds(5));
        let record = build_session_record(SessionRecordParts {
            token_hash: parse_token_hash("hash"),
            user_id: UserId::from(1),
            username: parse_username("alice"),
            label: parse_session_label("label"),
            created_at: now.into(),
            last_used_at: later.into(),
        });
        assert_eq!(record.token_hash, "hash");
        assert_eq!(record.username, "alice");
    }

    #[test]
    fn serialized_post_tags_accept_empty_tags() {
        let tags = SerializedPostTags::from_str("[]")
            .unwrap()
            .into_tags(PostId::from(10));
        assert!(tags.is_empty());
    }

    #[test]
    fn serialized_post_tags_display_encodes_canonical_json() {
        let tags = SerializedPostTags::from_str(
            r#"[{"tag_id": 1, "tag_slug": "rust", "tag_display": "Rust"}]"#,
        )
        .unwrap();

        assert_eq!(
            tags.0.to_string(),
            r#"[{"tag_id":1,"tag_slug":"rust","tag_display":"Rust"}]"#
        );
    }

    #[test]
    fn serialized_post_tags_attach_post_identity_after_parsing() {
        let tags_json = r#"[{"tag_id": 1, "tag_slug": "rust", "tag_display": "Rust"}]"#;
        let tags = SerializedPostTags::from_str(tags_json)
            .unwrap()
            .into_tags(PostId::from(10));

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].post_id, PostId::from(10));
        assert_eq!(tags[0].tag_id, TagId::from(1));
        assert_eq!(tags[0].tag_slug, "rust");
        assert_eq!(tags[0].tag_display, "Rust");
    }

    #[test]
    fn serialized_post_tags_reject_invalid_json() {
        assert!(SerializedPostTags::from_str("not-json").is_err());
    }

    #[test]
    fn serialized_post_tags_reject_invalid_tag_slug() {
        let tags_json = r#"[{"tag_id": 1, "tag_slug": "Not A Slug", "tag_display": "Bad"}]"#;
        assert!(SerializedPostTags::from_str(tags_json).is_err());
    }

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn test_hash_and_verify_password() {
        let password: password::Password = host::test_support::parse_password("password123");
        // No `.parse()` here: `hash_password` returns the newtype, per ADR-0063 §5. A
        // re-derive at the assignment is exactly what that section forbids.
        let hash = hash_password(password.clone()).await.unwrap();

        assert!(
            verify_password_with(password.clone(), hash.clone(), password::verify,)
                .await
                .unwrap()
        );
        assert!(
            !verify_password_with(
                host::test_support::parse_password("other-pass"),
                hash,
                password::verify,
            )
            .await
            .unwrap()
        );
    }

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn test_verify_password_rejects_invalid_hash() {
        // `StoredPasswordHash` accepts this — its invariant is only non-emptiness, so
        // argon2 remains the single arbiter of whether a stored string is a usable hash.
        let malformed: StoredPasswordHash = "not-a-hash".parse().expect("non-empty");
        let err = verify_password_with(
            host::test_support::parse_password("password123"),
            malformed,
            password::verify,
        )
        .await
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    // guard:no-backend — injected password hashing failure; no database
    #[tokio::test]
    async fn hash_password_preserves_password_error_source_through_io() {
        let password = host::test_support::parse_password("password123");
        let expected = forced_hash_failure(&password).unwrap_err();
        let error = hash_password_with(password, forced_hash_failure)
            .await
            .unwrap_err();
        let source = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<password::PasswordError>())
            .expect("io error retains PasswordError");

        let (
            password::PasswordError::HashingFailed(actual),
            password::PasswordError::HashingFailed(expected),
        ) = (source, &expected)
        else {
            panic!("expected typed hashing failures"); // cov:ignore
        };
        assert_eq!(actual, expected);
    }

    // guard:no-backend — injected password verification failure; no database
    #[tokio::test]
    async fn verify_password_preserves_password_error_source_through_io() {
        let password = host::test_support::parse_password("password123");
        let hash = fallback_dummy_password_hash();
        let expected = forced_verify_failure(&password, hash.as_ref()).unwrap_err();
        let error = verify_password_with(password, hash, forced_verify_failure)
            .await
            .unwrap_err();
        let source = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<password::PasswordError>())
            .expect("io error retains PasswordError");

        let (
            password::PasswordError::VerificationFailed(actual),
            password::PasswordError::VerificationFailed(expected),
        ) = (source, &expected)
        else {
            panic!("expected typed verification failures"); // cov:ignore
        };
        assert_eq!(actual, expected);
    }
    #[test]
    fn continuation_reporting_dummy_password_hash_failure_installs_fallback_and_reports_once() {
        let hash = std::sync::OnceLock::new();
        let (actual, trace) =
            swallowed_test::capture(|| dummy_password_hash_with(&hash, forced_hash_failure));
        let expected = fallback_dummy_password_hash();
        assert_eq!(actual.as_ref(), expected.as_ref());
        swallowed_test::assert_one_report(&trace, "storage.auth.dummy_password_hash");
        assert!(!trace.contains(DUMMY_PASSWORD), "trace leaked dummy secret");
    }

    // `build_user_record` and `build_session_record` have nothing to parse: their
    // string columns decode straight into newtypes via the sqlx bridge (#438), so
    // a malformed stored value is a `ColumnDecode` error at the query boundary,
    // covered by `users.rs`'s / `sessions.rs`'s decode-error tests.

    #[test]
    fn session_row_helper_round_trips() {
        let now = UtcInstant::now();
        let last_used_at = UtcInstant::from(now.value() + chrono::Duration::seconds(5));
        let session = SessionRow {
            token_hash: parse_token_hash("tokenhash"),
            user_id: UserId::from(1),
            username: parse_username("alice"),
            label: StoredSessionLabel("label".to_owned()),
            created_at: now.into(),
            last_used_at: last_used_at.into(),
        };
        let session_record = session_record_from_row(session);
        assert_eq!(session_record.user_id, UserId::from(1));
        assert_eq!(session_record.created_at, now);
        assert_eq!(session_record.last_used_at, last_used_at);
    }

    #[test]
    fn token_state_classifier_distinguishes_all_arms() {
        let now: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
        let expired_at: UtcInstant = "2099-01-02T03:04:05.123455Z".parse().unwrap();
        let claimable_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
        let used_at: UtcInstant = "2099-01-02T03:04:05.123454Z".parse().unwrap();

        assert_eq!(classify_token_state(None, now), TokenState::Missing);
        assert_eq!(
            classify_token_state(Some((Some(used_at), claimable_at)), now),
            TokenState::AlreadyUsed
        );
        assert_eq!(
            classify_token_state(Some((None, now)), now),
            TokenState::Expired
        );
        assert_eq!(
            classify_token_state(Some((None, expired_at)), now),
            TokenState::Expired
        );
        assert_eq!(
            classify_token_state(Some((None, claimable_at)), now),
            TokenState::Claimable
        );
    }

    #[test]
    fn invite_token_state_classifier_preserves_roles_and_exact_expiry() {
        let now: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
        let expired_at: UtcInstant = "2099-01-02T03:04:05.123455Z".parse().unwrap();
        let claimable_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
        let used_at: UtcInstant = "2099-01-02T03:04:05.123454Z".parse().unwrap();

        assert_eq!(classify_invite_token_state(None, now), TokenState::Missing);
        assert_eq!(
            classify_invite_token_state(Some((Some(used_at), claimable_at)), now),
            TokenState::AlreadyUsed
        );
        assert_eq!(
            classify_invite_token_state(Some((None, now)), now),
            TokenState::Expired
        );
        assert_eq!(
            classify_invite_token_state(Some((None, expired_at)), now),
            TokenState::Expired
        );
        assert_eq!(
            classify_invite_token_state(Some((None, claimable_at)), now),
            TokenState::Claimable
        );
    }

    #[test]
    fn email_verification_claim_error_distinguishes_all_arms() {
        let now: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
        let used_at: UtcInstant = "2099-01-02T03:04:05.123455Z".parse().unwrap();
        assert!(matches!(
            email_verification_claim_error(None, now),
            crate::UseEmailVerificationError::NotFound
        ));
        assert!(matches!(
            email_verification_claim_error(Some((Some(used_at), now)), now),
            crate::UseEmailVerificationError::AlreadyUsed
        ));
        assert!(matches!(
            email_verification_claim_error(Some((None, now)), now),
            crate::UseEmailVerificationError::Expired
        ));
    }

    #[test]
    fn password_reset_claim_error_distinguishes_all_arms() {
        let now: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
        let used_at: UtcInstant = "2099-01-02T03:04:05.123455Z".parse().unwrap();
        assert!(matches!(
            password_reset_claim_error(None, now),
            crate::UsePasswordResetError::NotFound
        ));
        assert!(matches!(
            password_reset_claim_error(Some((Some(used_at), now)), now),
            crate::UsePasswordResetError::AlreadyUsed
        ));
        assert!(matches!(
            password_reset_claim_error(Some((None, now)), now),
            crate::UsePasswordResetError::Expired
        ));
    }

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn dummy_password_hash_is_a_valid_verifiable_hash() {
        // The absent-user authentication path verifies against this hash to
        // equalize timing (§2.1). It must be a well-formed Argon2 hash so the
        // verification does real work and returns Ok(false) for a non-matching
        // password — not a fast Err that would reintroduce a timing oracle.
        let wrong = host::test_support::parse_password("definitely-not-the-dummy");
        let result = verify_password_with(wrong, dummy_password_hash().clone(), password::verify)
            .await
            .expect("dummy hash must be well-formed");
        assert!(!result, "a non-matching password must verify to false");
    }

    // guard:no-backend — password verification against a constant; no database
    #[tokio::test]
    async fn fallback_dummy_password_hash_is_a_valid_verifiable_hash() {
        // The fallback is only taken if runtime Argon2 hashing fails, so nothing else
        // exercises it — yet it must be a well-formed Argon2 hash for the same reason the
        // runtime one must: a fast `Err` on the absent-user path would reintroduce the
        // timing oracle this mechanism exists to close
        // (docs/adr/0114-absent-user-timing-equalization.md).
        let wrong = host::test_support::parse_password("definitely-not-the-dummy");
        let result = verify_password_with(wrong, fallback_dummy_password_hash(), password::verify)
            .await
            .expect("the fallback hash constant must be well-formed");
        assert!(!result, "a non-matching password must verify to false");
    }

    // No parameter-parity test for the fallback, deliberately: the constant carries
    // production parameters, which a `cheap-kdf` build cannot match
    // (docs/adr/0114-absent-user-timing-equalization.md).

    #[test]
    fn dummy_password_hash_matches_real_hash_parameters() {
        // Timing parity requires the dummy hash to carry the same Argon2
        // parameters as real password hashes (verify cost is derived from the
        // hash string's encoded params).
        let real = password::hash(&host::test_support::parse_password("some-real-password"))
            .expect("hashing succeeds");
        // PHC format: $argon2id$v=19$<params>$<salt>$<hash>
        let params = |h: &str| h.split('$').nth(3).map(str::to_owned);
        assert_eq!(params(dummy_password_hash().as_ref()), params(&real));
    }
}
