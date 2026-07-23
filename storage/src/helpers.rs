//! Helper functions for row type conversions and cryptographic operations.

use std::io;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    InviteRecord, MediaRecord, PostFormat, PostRecord, PostTag, RenderedHtml, SessionRecord,
    UserRecord,
};
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::ids::{PostId, TagId, UserId};
use common::media::{ByteSize, ContentHash, ContentType, Filename, MediaSource};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::token::TokenHash;
use common::username::Username;
use host::invite::InviteCode;

// ---------------------------------------------------------------------------
// UserRecord helpers
// ---------------------------------------------------------------------------

pub(crate) type UserRecordParts = (
    i64,
    Username,
    Option<DisplayName>,
    Option<Bio>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<Email>,
    bool,
    bool,
);

pub(crate) fn build_user_record(
    (
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
        is_operator,
    ): UserRecordParts,
) -> UserRecord {
    // The `username`, `display_name`, and `email` columns decode straight into
    // their domain newtypes via the sqlx bridge (#438), which validates through
    // `FromStr`, so a corrupt/migrated value is rejected as a column-decode error
    // before we ever get here — this build step is now infallible.
    UserRecord {
        user_id: UserId::from(user_id),
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

pub(crate) fn build_session_record(
    token_hash: TokenHash,
    user_id: i64,
    username: Username,
    label: String,
    created_at: DateTime<Utc>,
    last_used_at: DateTime<Utc>,
) -> SessionRecord {
    // The `token_hash` and `username` columns decode straight into their domain
    // newtypes via the sqlx bridge (#438), which validates through `FromStr`, so a
    // corrupt/migrated value is rejected as a column-decode error before we ever
    // get here — this build step is now infallible.
    SessionRecord {
        token_hash,
        user_id: UserId::from(user_id),
        username,
        label,
        created_at,
        last_used_at,
    }
}

// ---------------------------------------------------------------------------
// InviteRecord helpers
// ---------------------------------------------------------------------------

pub(crate) fn build_invite_record(
    code: InviteCode,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
    used_by: Option<i64>,
) -> InviteRecord {
    // The `code` column decodes straight into `InviteCode` via the sqlx bridge (#438),
    // which validates through `FromStr`, so a corrupt/migrated value is rejected as a
    // decode error before we ever get here — this build step is now infallible.
    InviteRecord {
        code,
        created_at,
        expires_at,
        used_at,
        used_by: used_by.map(UserId::from),
    }
}

// ---------------------------------------------------------------------------
// PostRecord helpers
// ---------------------------------------------------------------------------

// `author_username`/`title`/`slug`/`body`/`format` decode straight into their domain
// types via the sqlx bridge (the newtypes via #438, `PostFormat` via its text-enum
// bridge, #572). `rendered_html` (`RenderedHtml`) has a deliberately *write-only* sqlx
// bridge (#502: `Type`/`Encode`, no `Decode` — a `Decode` would bless any text column
// as trusted HTML), so its column decodes as a `String` here and is rebuilt via the
// gated `from_trusted` in `build_post_record`.
pub(crate) type PostRecordParts = (
    i64,
    i64,
    Username,
    Option<PostTitle>,
    Slug,
    PostBody,
    PostFormat,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<PostSummary>,
    String,
);

/// Row shape for the JSON-aggregated tags column. Field names match the SQL
/// `json_object` keys verbatim, hence the matching `tag_` prefixes.
// Fields mirror the SQL `json_object` aggregation keys (tag_id/tag_slug/tag_display)
// this struct deserializes; renaming would need per-field `#[serde(rename)]` for no gain.
#[expect(clippy::struct_field_names)]
#[derive(Deserialize)]
struct PostTagJson {
    tag_id: TagId,
    tag_slug: Tag,
    tag_display: TagLabel,
}

fn parse_post_tags_json(json: &str, post_id: PostId) -> sqlx::Result<Vec<PostTag>> {
    // `Tag`/`TagLabel` validate on deserialize (the serde bridge), so this is a
    // straight field-move: an invalid stored slug or label surfaces as a decode
    // error from `from_str` above.
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

pub(crate) fn build_post_record(
    (
        post_id,
        user_id,
        author_username,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        updated_at,
        published_at,
        deleted_at,
        summary,
        tags_json,
    ): PostRecordParts,
) -> sqlx::Result<PostRecord> {
    // `author_username`, `title`, `slug`, `body`, and `format` already arrived as
    // their domain types — the sqlx bridge decoded each column (the newtypes via #438,
    // `format` via its `PostFormat` text-enum bridge, #572), so a corrupt/migrated
    // value is rejected as a column-decode error before we get here. The JSON `tags`
    // still parse here, so this step stays fallible.
    let post_id = PostId::from(post_id);
    let tags = parse_post_tags_json(&tags_json, post_id)?;

    Ok(PostRecord {
        post_id,
        user_id: UserId::from(user_id),
        author_username,
        title,
        slug,
        body,
        format,
        // Trusted rebuild: this column only ever holds prior `render()` output.
        rendered_html: RenderedHtml::from_trusted(rendered_html),
        created_at,
        updated_at,
        published_at,
        deleted_at,
        summary,
        tags,
    })
}

// ---------------------------------------------------------------------------
// Row types and conversions
// ---------------------------------------------------------------------------

pub(crate) type UserRow = (
    i64,
    Username,
    Option<DisplayName>,
    Option<Bio>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<Email>,
    bool,
    bool,
);

pub(crate) fn user_record_from_row(row: UserRow) -> UserRecord {
    build_user_record(row)
}

pub(crate) type SessionRow = (
    TokenHash,
    i64,
    Username,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
);

pub(crate) fn session_record_from_row(row: SessionRow) -> SessionRecord {
    let (token_hash, user_id, username, label, created_at, last_used_at) = row;
    build_session_record(
        token_hash,
        user_id,
        username,
        label,
        created_at,
        last_used_at,
    )
}

pub(crate) type InviteRow = (
    InviteCode,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<i64>,
);

pub(crate) fn invite_record_from_row(row: InviteRow) -> InviteRecord {
    let (code, created_at, expires_at, used_at, used_by) = row;
    build_invite_record(code, created_at, expires_at, used_at, used_by)
}

// Mirrors [`PostRecordParts`]: the `username`/`title`/`slug`/`body` columns
// decode straight into their newtypes via the sqlx bridge (#438); `format` and
// `rendered_html` stay `String` (see the `PostRecordParts` note).
pub(crate) type PostRow = (
    i64,
    i64,
    Username,
    Option<PostTitle>,
    Slug,
    PostBody,
    PostFormat,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<PostSummary>,
    String,
);

pub(crate) fn post_record_from_row(row: PostRow) -> sqlx::Result<PostRecord> {
    build_post_record(row)
}

pub(crate) type MediaRow = (
    i64,
    ContentHash,
    Filename,
    String,
    ContentType,
    i64,
    Option<String>,
    DateTime<Utc>,
);

pub(crate) fn media_record_from_row(row: MediaRow) -> sqlx::Result<MediaRecord> {
    let (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at) = row;
    // `sha256` and `filename` already arrived as their domain newtypes — the sqlx
    // bridge (#438) decoded each column through its validating `FromStr`, so a
    // corrupt or hand-edited value is rejected as a column-decode error before we
    // get here (was a hand re-parse). `source` (a `MediaSource` enum) still parses
    // here, so this step stays fallible.
    let source: MediaSource = source
        .parse()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    // `size_bytes` arrives as the raw `i64` column; route it through the checked
    // door so a negative stored value is rejected as a column-decode error (mirrors
    // the `source` parse above).
    let size_bytes = ByteSize::try_from(size_bytes).map_err(|e| sqlx::Error::ColumnDecode {
        index: "size_bytes".into(),
        source: Box::new(e),
    })?;
    Ok(MediaRecord {
        user_id: UserId::from(user_id),
        sha256,
        filename,
        source,
        content_type,
        size_bytes,
        source_url,
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Claim verification error helpers
// ---------------------------------------------------------------------------

pub(crate) fn email_verification_claim_error(
    row: Option<(Option<DateTime<Utc>>, DateTime<Utc>)>,
) -> crate::UseEmailVerificationError {
    match row {
        None => crate::UseEmailVerificationError::NotFound,
        Some((Some(_), _)) => crate::UseEmailVerificationError::AlreadyUsed,
        Some((None, _)) => crate::UseEmailVerificationError::Expired,
    }
}

pub(crate) fn password_reset_claim_error(
    row: Option<(Option<DateTime<Utc>>, DateTime<Utc>)>,
) -> crate::UsePasswordResetError {
    match row {
        None => crate::UsePasswordResetError::NotFound,
        Some((Some(_), _)) => crate::UsePasswordResetError::AlreadyUsed,
        Some((None, _)) => crate::UsePasswordResetError::Expired,
    }
}

// ---------------------------------------------------------------------------
// Cryptographic operations
// ---------------------------------------------------------------------------

#[tracing::instrument(name = "crypto.password.hash", skip(password))]
pub(crate) async fn hash_password(password: common::password::Password) -> io::Result<String> {
    // Test-only hash-failure injection. Gated on `test` (storage's own unit tests) OR the
    // `test-utils` feature (enabled by `server`'s dev-dependencies) so the cross-backend
    // integration tests can exercise the `Internal` / validate-before-hash paths too;
    // absent from production builds, which enable neither.
    #[cfg(any(test, feature = "test-utils"))]
    if password.as_ref() == "force-hash-error-for-test-coverage" {
        return Err(io::Error::other("forced hash error"));
    }

    tokio::task::spawn_blocking(move || password.hash())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

/// Throwaway password hashed once to seed [`dummy_password_hash`].
const DUMMY_PASSWORD: &str = "jaunder-timing-equalization-dummy";

/// Valid Argon2id hash (default parameters) used only if runtime hashing of
/// [`DUMMY_PASSWORD`] ever fails, so initialization stays infallible (no
/// `unwrap`/`expect` in production). Regenerate with the same parameters as
/// `common::password::Password::hash` if the Argon2 defaults change.
const DUMMY_PASSWORD_HASH_FALLBACK: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$MlXSqqFgPKBHXn92Klja9Q$FCo2fJCKGcEhWHiq+R7lVdfcP/TpFgrVKfK6bMoB3CM";

/// Returns a fixed, valid Argon2id hash used to equalize authentication timing
/// on the absent-user path, mitigating username enumeration via timing (see
/// analysis §2.1).
///
/// `authenticate` runs a full Argon2 verification only when the username
/// exists; an attacker can otherwise distinguish "no such user" (fast) from
/// "wrong password" (slow). The absent path verifies against this hash so both
/// outcomes take the same time. It is computed once with the same default
/// Argon2 parameters as real password hashes (`Password::hash`), so the dummy
/// verification costs the same as a genuine one.
pub(crate) fn dummy_password_hash() -> &'static str {
    use common::password::Password;
    use std::str::FromStr;
    use std::sync::OnceLock;

    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        Password::from_str(DUMMY_PASSWORD)
            .ok()
            .and_then(|p| p.hash().ok())
            .unwrap_or_else(|| DUMMY_PASSWORD_HASH_FALLBACK.to_string())
    })
}

#[tracing::instrument(name = "crypto.password.verify", skip(password, hash))]
pub(crate) async fn verify_password(
    password: common::password::Password,
    hash: String,
) -> io::Result<bool> {
    #[cfg(test)]
    if password.as_ref() == "force-verify-error-for-test-coverage" {
        return Err(io::Error::other("forced verify error"));
    }

    tokio::task::spawn_blocking(move || password.verify(&hash))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_invite_code;
    use chrono::Utc;
    use common::test_support::{
        parse_bio, parse_content_hash, parse_content_type, parse_display_name, parse_email,
        parse_filename, parse_password, parse_slug, parse_token_hash, parse_username,
    };

    #[test]
    fn test_build_user_record() {
        let now = Utc::now();
        let parts: UserRecordParts = (
            1,
            parse_username("alice"),
            Some(parse_display_name("Alice")),
            Some(parse_bio("Bio")),
            now,
            Some(now),
            Some(parse_email("alice@example.com")),
            true,
            false,
        );
        let record = build_user_record(parts);
        assert_eq!(record.user_id, UserId::from(1));
        assert_eq!(record.username, "alice");
        assert_eq!(record.email.unwrap(), "alice@example.com");
    }

    #[test]
    fn test_build_session_record() {
        let now = Utc::now();
        let record = build_session_record(
            parse_token_hash("hash"),
            1,
            parse_username("alice"),
            "label".to_string(),
            now,
            now,
        );
        assert_eq!(record.token_hash, "hash");
        assert_eq!(record.username, "alice");
    }

    #[test]
    fn test_build_invite_record() {
        let created_at = Utc::now();
        let expires_at = created_at + chrono::Duration::days(7);
        let used_at = created_at + chrono::Duration::hours(1);
        let record = build_invite_record(
            parse_invite_code("invite-code"),
            created_at,
            expires_at,
            Some(used_at),
            Some(7),
        );

        assert_eq!(record.code.as_ref(), "invite-code");
        assert_eq!(record.created_at, created_at);
        assert_eq!(record.expires_at, expires_at);
        assert_eq!(record.used_at, Some(used_at));
        assert_eq!(record.used_by, Some(UserId::from(7)));
    }

    #[test]
    fn test_build_post_record() {
        let now = Utc::now();
        let record = build_post_record((
            10,
            20,
            parse_username("alice"),
            Some("Hello".into()),
            parse_slug("hello-world"),
            "Body".into(),
            PostFormat::Markdown,
            "<p>Body</p>".to_string(),
            now,
            now,
            Some(now),
            None,
            None,
            "[]".to_string(),
        ))
        .unwrap();

        assert_eq!(record.post_id, PostId::from(10));
        assert_eq!(record.user_id, UserId::from(20));
        assert_eq!(record.author_username, "alice");
        assert_eq!(record.slug, "hello-world");
        assert_eq!(record.format, PostFormat::Markdown);
        assert_eq!(record.published_at, Some(now));
        assert_eq!(record.deleted_at, None);
        assert!(record.tags.is_empty());
    }

    // `build_post_record` no longer parses `username`/`slug`/`format`: they decode
    // straight into `Username`/`Slug`/`PostFormat` via the sqlx bridge (the newtypes
    // via #438, `PostFormat` via its text-enum bridge #572), so a malformed stored
    // value is rejected as a `ColumnDecode` error at the query boundary (covered by
    // `posts.rs`'s decode-error tests), not here. Only the JSON `tags` still parse in
    // `build_post_record`, so those rejections stay below.

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn test_hash_and_verify_password() {
        let password: common::password::Password = parse_password("password123");
        let hash = hash_password(password.clone()).await.unwrap();

        assert!(verify_password(password.clone(), hash.clone())
            .await
            .unwrap());
        assert!(!verify_password(parse_password("other-pass"), hash)
            .await
            .unwrap());
    }

    // guard:no-backend — password hashing/verification; no database
    #[tokio::test]
    async fn test_verify_password_rejects_invalid_hash() {
        let err = verify_password(parse_password("password123"), "not-a-hash".to_string())
            .await
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    // `build_user_record` no longer parses: `username`/`display_name`/`email`
    // decode straight into their newtypes via the sqlx bridge (#438), so a
    // malformed stored value is rejected as a `ColumnDecode` error at the query
    // boundary (covered by `users.rs`'s decode-error tests), not here.

    // `build_session_record` no longer parses: `token_hash`/`username` decode
    // straight into their newtypes via the sqlx bridge (#438), so a malformed
    // stored value is rejected as a `ColumnDecode` error at the query boundary
    // (covered by `sessions.rs`'s decode-error test), not here.

    #[test]
    fn build_post_record_with_valid_tags_json_parses_tags() {
        let now = Utc::now();
        let tags_json = r#"[{"tag_id": 1, "tag_slug": "rust", "tag_display": "Rust"}]"#;
        let record = build_post_record((
            10,
            20,
            parse_username("alice"),
            None,
            parse_slug("hello-world"),
            "Body".into(),
            PostFormat::Markdown,
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
            None,
            tags_json.to_string(),
        ))
        .unwrap();
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_id, TagId::from(1));
        assert_eq!(record.tags[0].tag_slug, "rust");
        assert_eq!(record.tags[0].tag_display, "Rust");
    }

    #[test]
    fn build_post_record_rejects_invalid_tags_json() {
        let now = Utc::now();
        let err = build_post_record((
            10,
            20,
            parse_username("alice"),
            None,
            parse_slug("hello-world"),
            "Body".into(),
            PostFormat::Markdown,
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
            None,
            "not-json".to_string(),
        ))
        .unwrap_err();
        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    #[test]
    fn build_post_record_rejects_invalid_tag_slug_in_json() {
        let now = Utc::now();
        let tags_json =
            r#"[{"tag_id": 1, "tag_slug": "Not A Slug", "tag_display": "Bad"}]"#.to_string();
        let err = build_post_record((
            10,
            20,
            parse_username("alice"),
            None,
            parse_slug("hello-world"),
            "Body".into(),
            PostFormat::Markdown,
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
            None,
            tags_json,
        ))
        .unwrap_err();
        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    /// A canonical 64-char lowercase-hex content hash for row fixtures.
    const ROW_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn media_record_from_row_rejects_invalid_source() {
        let row: MediaRow = (
            1,
            parse_content_hash(ROW_HASH),
            parse_filename("file.png"),
            "not-a-source".to_string(),
            parse_content_type("image/png"),
            42,
            None,
            Utc::now(),
        );
        let err = media_record_from_row(row).unwrap_err();
        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    // `media_record_from_row` no longer hand-parses `sha256`/`filename`: those columns
    // decode straight into `ContentHash`/`Filename` via the sqlx bridge (#438), so a
    // malformed stored value is rejected as a `ColumnDecode` error at the query boundary
    // (covered by `media.rs`'s decode-error test), not here — a `MediaRow` cannot even
    // hold an invalid value. Only `source` (a `MediaSource` enum) still parses here.

    #[test]
    fn media_record_from_row_accepts_valid_source() {
        let row: MediaRow = (
            1,
            parse_content_hash(ROW_HASH),
            parse_filename("file.png"),
            "upload".to_string(),
            parse_content_type("image/png"),
            42,
            None,
            Utc::now(),
        );
        let record = media_record_from_row(row).unwrap();
        assert_eq!(record.user_id, UserId::from(1));
        assert_eq!(record.source, MediaSource::Upload);
        assert_eq!(record.sha256, ROW_HASH);
        assert_eq!(record.filename, "file.png");
    }

    #[test]
    fn session_and_invite_row_helpers_round_trip() {
        let now = Utc::now();
        let session: SessionRow = (
            parse_token_hash("tokenhash"),
            1,
            parse_username("alice"),
            "label".to_string(),
            now,
            now,
        );
        let session_record = session_record_from_row(session);
        assert_eq!(session_record.user_id, UserId::from(1));

        let invite: InviteRow = (parse_invite_code("code"), now, now, None, None);
        let invite_record = invite_record_from_row(invite);
        assert_eq!(invite_record.code.as_ref(), "code");
    }

    #[test]
    fn user_row_helper_delegates_to_build_user_record() {
        let now = Utc::now();
        let row: UserRow = (
            1,
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
    fn post_row_helper_delegates_to_build_post_record() {
        let now = Utc::now();
        let row: PostRow = (
            10,
            20,
            parse_username("alice"),
            None,
            parse_slug("hello-world"),
            "Body".into(),
            PostFormat::Markdown,
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
            None,
            "[]".to_string(),
        );
        let record = post_record_from_row(row).unwrap();
        assert_eq!(record.post_id, PostId::from(10));
    }

    #[test]
    fn email_verification_claim_error_distinguishes_all_arms() {
        let now = Utc::now();
        assert!(matches!(
            email_verification_claim_error(None),
            crate::UseEmailVerificationError::NotFound
        ));
        assert!(matches!(
            email_verification_claim_error(Some((Some(now), now))),
            crate::UseEmailVerificationError::AlreadyUsed
        ));
        assert!(matches!(
            email_verification_claim_error(Some((None, now))),
            crate::UseEmailVerificationError::Expired
        ));
    }

    #[test]
    fn password_reset_claim_error_distinguishes_all_arms() {
        let now = Utc::now();
        assert!(matches!(
            password_reset_claim_error(None),
            crate::UsePasswordResetError::NotFound
        ));
        assert!(matches!(
            password_reset_claim_error(Some((Some(now), now))),
            crate::UsePasswordResetError::AlreadyUsed
        ));
        assert!(matches!(
            password_reset_claim_error(Some((None, now))),
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
        let wrong = parse_password("definitely-not-the-dummy");
        let result = verify_password(wrong, dummy_password_hash().to_string())
            .await
            .expect("dummy hash must be well-formed");
        assert!(!result, "a non-matching password must verify to false");
    }

    #[test]
    fn dummy_password_hash_matches_real_hash_parameters() {
        // Timing parity requires the dummy hash to carry the same Argon2
        // parameters as real password hashes (verify cost is derived from the
        // hash string's encoded params).
        let real = parse_password("some-real-password")
            .hash()
            .expect("hashing succeeds");
        // PHC format: $argon2id$v=19$<params>$<salt>$<hash>
        let params = |h: &str| h.split('$').nth(3).map(str::to_owned);
        assert_eq!(params(dummy_password_hash()), params(&real));
    }

    #[test]
    fn user_record_from_row_maps_some_fields() {
        let now = Utc::now();
        let row: UserRow = (
            1,
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
        let now = Utc::now();
        let row: InviteRow = (parse_invite_code("code"), now, now, Some(now), Some(1));
        let record = invite_record_from_row(row);
        assert_eq!(record.code.as_ref(), "code");
        assert_eq!(record.created_at, now);
        assert_eq!(record.expires_at, now);
        assert_eq!(record.used_at, Some(now));
        assert_eq!(record.used_by, Some(UserId::from(1)));
    }
}
