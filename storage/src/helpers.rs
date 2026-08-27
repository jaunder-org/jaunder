//! Helper functions for row type conversions and cryptographic operations.

use std::io;

use serde::Deserialize;

use crate::role_instant::impl_role_instant;
use crate::{InviteRecord, MediaRecord, PostTag, SessionRecord, UserRecord};
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::ids::{PostId, TagId, UserId};
use common::media::{ByteSize, ContentHash, ContentType, Filename, MediaSource};
use common::session_label::SessionLabel;
use common::tag::{Tag, TagLabel};
use common::tagged_url::MediaSourceUrl;
use common::time::UtcInstant;
use common::token::TokenHash;
use common::username::Username;
use host::invite::InviteCode;
use host::stored_password_hash::StoredPasswordHash;

/// The `sessions.created_at` storage timestamp role, distinct from
/// `last_used_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct SessionCreatedAt(UtcInstant);
impl_role_instant!(SessionCreatedAt, UtcInstant);

/// The `sessions.last_used_at` storage timestamp role, distinct from
/// `created_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct SessionLastUsedAt(UtcInstant);
impl_role_instant!(SessionLastUsedAt, UtcInstant);

/// The `invites.created_at` storage timestamp role, distinct from `expires_at`
/// so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct InviteCreatedAt(UtcInstant);
impl_role_instant!(InviteCreatedAt, UtcInstant);

/// The `invites.expires_at` storage timestamp role, distinct from `created_at`
/// so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct InviteExpiresAt(UtcInstant);
impl_role_instant!(InviteExpiresAt, UtcInstant);

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

/// The parts a [`UserRecord`] is assembled from.
///
/// **Named fields, not a tuple.** `email_verified` and `is_operator` are adjacent
/// `bool`s: as a positional tuple, swapping them compiled silently and turned a verified
/// flag into an operator grant. Naming them makes a swap visible at the one place the
/// mapping happens ([`user_record_from_row`]) instead of spreading a positional contract
/// across every caller.
///
/// Not a decode target and deliberately **not** `#[derive(FromRow)]` — [`UserRow`] is the
/// type rows decode into, and it stays a tuple alias precisely so the gate keeps policing
/// its elements.
pub(crate) struct UserRecordParts {
    pub(crate) user_id: UserId,
    pub(crate) username: Username,
    pub(crate) display_name: Option<DisplayName>,
    pub(crate) bio: Option<Bio>,
    pub(crate) created_at: UtcInstant,
    pub(crate) last_authenticated_at: Option<UtcInstant>,
    pub(crate) email: Option<Email>,
    pub(crate) email_verified: bool,
    pub(crate) is_operator: bool,
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
// InviteRecord helpers
// ---------------------------------------------------------------------------

struct InviteRecordParts {
    code: InviteCode,
    created_at: InviteCreatedAt,
    expires_at: InviteExpiresAt,
    used_at: Option<UtcInstant>,
    used_by: Option<UserId>,
}

fn build_invite_record(
    InviteRecordParts {
        code,
        created_at,
        expires_at,
        used_at,
        used_by,
    }: InviteRecordParts,
) -> InviteRecord {
    // The `code` column decodes straight into `InviteCode` via the sqlx bridge (#438),
    // `used_by` through the id bridge (#686), and the created/expires pair through
    // distinct role wrappers (#751), so corrupt/migrated values are rejected before
    // we ever get here and timestamp swaps fail at the row-to-parts seam.
    InviteRecord {
        code,
        created_at: created_at.value(),
        expires_at: expires_at.value(),
        used_at,
        used_by,
    }
}

// ---------------------------------------------------------------------------
// Post tag JSON helper
// ---------------------------------------------------------------------------

/// Row shape for the JSON-aggregated tags column. Field names match the SQL
/// `json_object` keys verbatim, hence the matching `tag_` prefixes.
// Fields mirror the SQL `json_object` aggregation keys (tag_id/tag_slug/tag_display)
// this struct deserializes; renaming would need per-field `#[serde(rename)]` for no gain.
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#[expect(clippy::struct_field_names)]
#[derive(Deserialize)]
struct PostTagJson {
    tag_id: TagId,
    tag_slug: Tag,
    tag_display: TagLabel,
}

pub(crate) fn parse_post_tags_json(json: &str, post_id: PostId) -> sqlx::Result<Vec<PostTag>> {
    // `Tag`/`TagLabel` validate on deserialize (the serde bridge), so an invalid stored
    // slug or label surfaces as a decode error from `from_str` above.
    let raw: Vec<PostTagJson> =
        serde_json::from_str(json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(raw
        .into_iter()
        .map(|r| PostTag {
            post_id,
            tag_id: r.tag_id,
            tag_slug: r.tag_slug,
            tag_display: r.tag_display,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Row types and conversions
// ---------------------------------------------------------------------------

pub(crate) type UserRow = (
    UserId,
    Username,
    Option<DisplayName>,
    Option<Bio>,
    UtcInstant,
    Option<UtcInstant>,
    Option<Email>,
    bool,
    bool,
);

/// The single positional→named boundary for a decoded user row.
///
/// [`UserRow`] is a tuple because that is what `query_as` decodes into; this is the one
/// place its order is interpreted. Concentrating that here is the point of
/// [`UserRecordParts`] having named fields: a mis-ordered pair is visible in one
/// reviewable mapping rather than implied at every call site.
pub(crate) fn user_record_from_row(row: UserRow) -> UserRecord {
    let (
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
        is_operator,
    ) = row;

    build_user_record(UserRecordParts {
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
        is_operator,
    })
}

#[derive(sqlx::FromRow)]
pub struct SessionRow {
    token_hash: TokenHash,
    user_id: UserId,
    username: Username,
    label: String,
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
    // The `label` column decodes as a plain `String` and is sanitized into a
    // `SessionLabel` via the lossy constructor rather than a validating decode: a
    // label is a best-effort *display* value, so a pre-existing out-of-range row
    // (empty, over-long) is repaired on read instead of failing the whole
    // `list_sessions` query.
    build_session_record(SessionRecordParts {
        token_hash: row.token_hash,
        user_id: row.user_id,
        username: row.username,
        label: SessionLabel::from_lossy(&row.label),
        created_at: row.created_at,
        last_used_at: row.last_used_at,
    })
}

#[derive(sqlx::FromRow)]
pub(crate) struct InviteRow {
    code: InviteCode,
    created_at: InviteCreatedAt,
    expires_at: InviteExpiresAt,
    used_at: Option<UtcInstant>,
    used_by: Option<UserId>,
}

pub(crate) fn invite_record_from_row(row: InviteRow) -> InviteRecord {
    build_invite_record(InviteRecordParts {
        code: row.code,
        created_at: row.created_at,
        expires_at: row.expires_at,
        used_at: row.used_at,
        used_by: row.used_by,
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

pub(crate) type MediaRow = (
    UserId,
    ContentHash,
    Filename,
    MediaSource,
    ContentType,
    ByteSize,
    Option<MediaSourceUrl>,
    UtcInstant,
);

pub(crate) fn media_record_from_row(row: MediaRow) -> MediaRecord {
    let (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at) = row;
    // Every column arrives as its domain type — `sha256`/`filename` through the validating
    // string bridge (#438), `source` through its `MediaSource` text-enum bridge (#607),
    // `source_url` as a `MediaSourceUrl` (#675), `user_id` through the id bridge and
    // `size_bytes` through the *bound-checking* numeric bridge (#686), whose `Decode`
    // re-runs `ByteSize`'s `min` so a negative stored value is still rejected as a
    // column-decode error. Nothing is left to convert, so this build step is infallible.
    MediaRecord {
        user_id,
        sha256,
        filename,
        source,
        content_type,
        size_bytes,
        source_url,
        created_at,
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
pub(crate) async fn hash_password(
    password: host::password::Password,
) -> io::Result<StoredPasswordHash> {
    let operation = hash_password_operation(&password);

    hash_password_with(password, operation).await
}

pub(crate) type HashPasswordOperation =
    fn(&host::password::Password) -> Result<String, host::password::PasswordError>;

pub(crate) fn hash_password_operation(
    password: &host::password::Password,
) -> HashPasswordOperation {
    #[cfg(any(test, feature = "test-utils"))]
    if password.as_ref() == "force-hash-error-for-test-coverage" {
        return forced_hash_failure;
    }

    host::password::hash
}

pub(crate) async fn hash_password_with(
    password: host::password::Password,
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
    password: &host::password::Password,
) -> Result<String, host::password::PasswordError> {
    match host::password::verify(password, NON_PASSWORD_ARGON2_FAILURE_HASH) {
        Err(host::password::PasswordError::VerificationFailed(source)) => {
            Err(host::password::PasswordError::HashingFailed(source))
        }
        _ => unreachable!("the fixed test hash always produces a non-password Argon2 failure"),
    }
}

/// Throwaway password hashed once to seed [`dummy_password_hash`].
const DUMMY_PASSWORD: &str = "jaunder-timing-equalization-dummy";

/// Valid Argon2id hash (default parameters) used only if runtime hashing of
/// [`DUMMY_PASSWORD`] ever fails, so initialization stays infallible (no
/// `unwrap`/`expect` in production). Regenerate with the same parameters as
/// `host::password::hash` if the Argon2 defaults change.
const DUMMY_PASSWORD_HASH_FALLBACK: &str = "$argon2id$v=19$m=19456,t=2,p=1$MlXSqqFgPKBHXn92Klja9Q$FCo2fJCKGcEhWHiq+R7lVdfcP/TpFgrVKfK6bMoB3CM";

/// Returns a fixed, valid Argon2id hash used to equalize authentication timing
/// on the absent-user path, mitigating username enumeration via timing (see
/// analysis §2.1).
///
/// `authenticate` runs a full Argon2 verification only when the username
/// exists; an attacker can otherwise distinguish "no such user" (fast) from
/// "wrong password" (slow). The absent path verifies against this hash so both
/// outcomes take the same time. It is computed once with the same default
/// Argon2 parameters as [`host::password::hash`], so the dummy verification
/// costs the same as a genuine one.
pub(crate) fn dummy_password_hash() -> &'static StoredPasswordHash {
    use std::sync::OnceLock;

    static HASH: OnceLock<StoredPasswordHash> = OnceLock::new();
    dummy_password_hash_with(&HASH, host::password::hash)
}

fn dummy_password_hash_with(
    hash: &std::sync::OnceLock<StoredPasswordHash>,
    operation: HashPasswordOperation,
) -> &StoredPasswordHash {
    use host::password::Password;
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
            Err(_) => unreachable!("host::password::hash returns a valid stored password hash"),
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
    fn(&host::password::Password, &str) -> Result<bool, host::password::PasswordError>;

#[tracing::instrument(name = "crypto.password.verify", skip(password, hash, operation))]
pub(crate) async fn verify_password_with(
    password: host::password::Password,
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
    password: &host::password::Password,
    _: &str,
) -> Result<bool, host::password::PasswordError> {
    host::password::verify(password, NON_PASSWORD_ARGON2_FAILURE_HASH)
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
    use crate::test_support::{Backend, backends, parse_invite_code};

    use common::test_support::{
        parse_bio, parse_byte_size, parse_content_hash, parse_content_type, parse_display_name,
        parse_email, parse_filename, parse_session_label, parse_token_hash, parse_username,
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
                .bind(expected)
                .fetch_one(pool)
                .await
                .unwrap()
        });
        assert_eq!(actual, expected);

        let absent = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, Option<UtcInstant>>("SELECT $1")
                .bind(None::<UtcInstant>)
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
            email_verified: true,
            is_operator: false,
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
    fn test_build_invite_record() {
        let created_at = UtcInstant::now();
        let expires_at = UtcInstant::from(created_at.value() + chrono::Duration::days(7));
        let used_at = UtcInstant::from(created_at.value() + chrono::Duration::hours(1));
        let record = build_invite_record(InviteRecordParts {
            code: parse_invite_code("invite-code"),
            created_at: created_at.into(),
            expires_at: expires_at.into(),
            used_at: Some(used_at),
            used_by: Some(UserId::from(7)),
        });

        assert_eq!(record.code.as_ref(), "invite-code");
        assert_eq!(record.created_at, created_at);
        assert_eq!(record.expires_at, expires_at);
        assert_eq!(record.used_at, Some(used_at));
        assert_eq!(record.used_by, Some(UserId::from(7)));
    }

    #[test]
    fn parse_post_tags_json_accepts_empty_tags() {
        let tags = parse_post_tags_json("[]", PostId::from(10)).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn parse_post_tags_json_parses_tags() {
        let tags_json = r#"[{"tag_id": 1, "tag_slug": "rust", "tag_display": "Rust"}]"#;
        let tags = parse_post_tags_json(tags_json, PostId::from(10)).unwrap();

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].post_id, PostId::from(10));
        assert_eq!(tags[0].tag_id, TagId::from(1));
        assert_eq!(tags[0].tag_slug, "rust");
        assert_eq!(tags[0].tag_display, "Rust");
    }

    #[test]
    fn parse_post_tags_json_rejects_invalid_json() {
        let err = parse_post_tags_json("not-json", PostId::from(10)).unwrap_err();
        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    #[test]
    fn parse_post_tags_json_rejects_invalid_tag_slug() {
        let tags_json = r#"[{"tag_id": 1, "tag_slug": "Not A Slug", "tag_display": "Bad"}]"#;
        let err = parse_post_tags_json(tags_json, PostId::from(10)).unwrap_err();
        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn test_hash_and_verify_password() {
        let password: host::password::Password = host::test_support::parse_password("password123");
        // No `.parse()` here: `hash_password` returns the newtype, per ADR-0063 §5. A
        // re-derive at the assignment is exactly what that section forbids.
        let hash = hash_password(password.clone()).await.unwrap();

        assert!(
            verify_password_with(password.clone(), hash.clone(), host::password::verify,)
                .await
                .unwrap()
        );
        assert!(
            !verify_password_with(
                host::test_support::parse_password("other-pass"),
                hash,
                host::password::verify,
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
            host::password::verify,
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
            .and_then(|source| source.downcast_ref::<host::password::PasswordError>())
            .expect("io error retains PasswordError");

        let (
            host::password::PasswordError::HashingFailed(actual),
            host::password::PasswordError::HashingFailed(expected),
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
            .and_then(|source| source.downcast_ref::<host::password::PasswordError>())
            .expect("io error retains PasswordError");

        let (
            host::password::PasswordError::VerificationFailed(actual),
            host::password::PasswordError::VerificationFailed(expected),
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

    /// A canonical 64-char lowercase-hex content hash for row fixtures.
    const ROW_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    // `media_record_from_row` has nothing to hand-parse: `sha256`/`filename`/`source`
    // decode straight into `ContentHash`/`Filename`/`MediaSource` via the sqlx bridge
    // (#438, #607), so a malformed stored value is a `ColumnDecode` error at the query
    // boundary (covered by `media.rs`'s decode-error tests) — a `MediaRow` cannot even
    // hold an invalid value.

    #[test]
    fn media_record_from_row_accepts_valid_source() {
        let row: MediaRow = (
            UserId::from(1),
            parse_content_hash(ROW_HASH),
            parse_filename("file.png"),
            MediaSource::Upload,
            parse_content_type("image/png"),
            parse_byte_size("42"),
            None,
            UtcInstant::now(),
        );
        let record = media_record_from_row(row);
        assert_eq!(record.user_id, UserId::from(1));
        assert_eq!(record.source, MediaSource::Upload);
        assert_eq!(record.sha256, ROW_HASH);
        assert_eq!(record.filename, "file.png");
    }

    #[test]
    fn session_and_invite_row_helpers_round_trip() {
        let now = UtcInstant::now();
        let last_used_at = UtcInstant::from(now.value() + chrono::Duration::seconds(5));
        let session = SessionRow {
            token_hash: parse_token_hash("tokenhash"),
            user_id: UserId::from(1),
            username: parse_username("alice"),
            label: "label".to_string(),
            created_at: now.into(),
            last_used_at: last_used_at.into(),
        };
        let session_record = session_record_from_row(session);
        assert_eq!(session_record.user_id, UserId::from(1));
        assert_eq!(session_record.created_at, now);
        assert_eq!(session_record.last_used_at, last_used_at);

        let expires_at = UtcInstant::from(now.value() + chrono::Duration::days(7));
        let invite = InviteRow {
            code: parse_invite_code("code"),
            created_at: now.into(),
            expires_at: expires_at.into(),
            used_at: None,
            used_by: None,
        };
        let invite_record = invite_record_from_row(invite);
        assert_eq!(invite_record.code.as_ref(), "code");
        assert_eq!(invite_record.created_at, now);
        assert_eq!(invite_record.expires_at, expires_at);
    }

    #[test]
    fn user_row_helper_delegates_to_build_user_record() {
        let now = UtcInstant::now();
        let row: UserRow = (
            UserId::from(1),
            parse_username("alice"),
            None,
            None,
            now,
            None,
            None,
            false,
            false,
        );
        let record = user_record_from_row(row);
        assert_eq!(record.user_id, UserId::from(1));
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
        let created_at: UtcInstant = "2099-01-01T03:04:05.123456Z".parse().unwrap();
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
        let parts = InviteRecordParts {
            code: parse_invite_code("role-ordering"),
            created_at: InviteCreatedAt::from(created_at),
            expires_at: InviteExpiresAt::from(claimable_at),
            used_at: None,
            used_by: None,
        };
        let record = build_invite_record(parts);
        assert_eq!(record.created_at, created_at);
        assert_eq!(record.expires_at, claimable_at);
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
        let result =
            verify_password_with(wrong, dummy_password_hash().clone(), host::password::verify)
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
        let result = verify_password_with(
            wrong,
            fallback_dummy_password_hash(),
            host::password::verify,
        )
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
        let real = host::password::hash(&host::test_support::parse_password("some-real-password"))
            .expect("hashing succeeds");
        // PHC format: $argon2id$v=19$<params>$<salt>$<hash>
        let params = |h: &str| h.split('$').nth(3).map(str::to_owned);
        assert_eq!(params(dummy_password_hash().as_ref()), params(&real));
    }

    #[test]
    fn user_record_from_row_maps_some_fields() {
        let now = UtcInstant::now();
        let row: UserRow = (
            UserId::from(1),
            parse_username("alice"),
            Some(parse_display_name("Alice")),
            Some(parse_bio("Bio")),
            now,
            Some(now),
            Some(parse_email("alice@example.com")),
            true,
            false,
        );
        let record = user_record_from_row(row);
        assert_eq!(record.user_id, UserId::from(1));
        assert_eq!(record.username, "alice");
        assert_eq!(record.display_name, Some(parse_display_name("Alice")));
        assert_eq!(record.bio, Some(parse_bio("Bio")));
        assert_eq!(record.created_at, now);
        assert_eq!(record.last_authenticated_at, Some(now));
        assert_eq!(record.email.unwrap(), "alice@example.com");
        assert!(record.email_verified);
    }

    #[test]
    fn invite_record_from_row_maps_some_fields() {
        let now = UtcInstant::now();
        let expires_at = UtcInstant::from(now.value() + chrono::Duration::days(7));
        let used_at = UtcInstant::from(now.value() + chrono::Duration::hours(1));
        let row = InviteRow {
            code: parse_invite_code("code"),
            created_at: now.into(),
            expires_at: expires_at.into(),
            used_at: Some(used_at),
            used_by: Some(UserId::from(1)),
        };
        let record = invite_record_from_row(row);
        assert_eq!(record.code.as_ref(), "code");
        assert_eq!(record.created_at, now);
        assert_eq!(record.expires_at, expires_at);
        assert_eq!(record.used_at, Some(used_at));
        assert_eq!(record.used_by, Some(UserId::from(1)));
    }
}
