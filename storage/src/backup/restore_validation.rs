use std::fmt;
use std::str::FromStr;

use crate::InstanceId;
use common::audience::AudienceName;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::idempotency_key::IdempotencyKey;
use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MediaReferenceForm, MediaReferenceKind,
    MediaSource,
};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::PostFormat;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::tagged_url::MediaSourceUrl;
use common::token::TokenHash;
use common::username::Username;
use common::visibility::{Channel, SubscriptionStatus, TargetKind};
use host::config_key::{SiteConfigKey, UserConfigKey};
use host::feed::{FeedEventStatus, FeedPath};
use host::invite::InviteCode;
use host::stored_password_hash::StoredPasswordHash;

use super::{
    RestoreText,
    format::{self, BackupManifest},
};

#[derive(Debug, Clone)]
pub struct BackupRestoreOutcome {
    pub manifest: BackupManifest,
    pub validation_report: RestoreValidationReport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreValidationReport {
    issues: Vec<RestoreValidationIssue>,
}

impl RestoreValidationReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    #[must_use]
    pub fn issues(&self) -> &[RestoreValidationIssue] {
        &self.issues
    }

    fn push(&mut self, issue: RestoreValidationIssue) {
        self.issues.push(issue);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreValidationIssue {
    pub table: String,
    pub column: String,
    pub value_class: String,
    pub reason: String,
}

impl fmt::Display for RestoreValidationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{} ({}): {}",
            self.table, self.column, self.value_class, self.reason
        )
    }
}

type RestoreRowMap = serde_json::Map<String, serde_json::Value>;

pub(crate) fn validate_restore_row(
    table: &str,
    row: &RestoreRowMap,
    report: &mut RestoreValidationReport,
) {
    match table {
        "audiences" => validate_typed_restore_row::<AudiencesRestoreRow>(row, report),
        "channels" => validate_typed_restore_row::<ChannelsRestoreRow>(row, report),
        "email_verifications" => {
            validate_typed_restore_row::<EmailVerificationsRestoreRow>(row, report);
        }
        "feed_events" => validate_typed_restore_row::<FeedEventsRestoreRow>(row, report),
        "idempotency_keys" => {
            validate_typed_restore_row::<IdempotencyKeysRestoreRow>(row, report);
        }
        "invites" => validate_typed_restore_row::<InvitesRestoreRow>(row, report),
        "media" => validate_typed_restore_row::<MediaRestoreRow>(row, report),
        "password_resets" => validate_typed_restore_row::<PasswordResetsRestoreRow>(row, report),
        "post_media" => validate_typed_restore_row::<PostMediaRestoreRow>(row, report),
        "post_revision_audiences" => {
            validate_typed_restore_row::<PostRevisionAudiencesRestoreRow>(row, report);
        }
        "post_revision_tags" => {
            validate_typed_restore_row::<PostRevisionTagsRestoreRow>(row, report);
        }
        "post_revisions" => validate_typed_restore_row::<PostRevisionsRestoreRow>(row, report),
        "post_tags" => validate_typed_restore_row::<PostTagsRestoreRow>(row, report),
        "posts" => validate_typed_restore_row::<PostsRestoreRow>(row, report),
        "sessions" => validate_typed_restore_row::<SessionsRestoreRow>(row, report),
        "site_config" => validate_typed_restore_row::<SiteConfigRestoreRow>(row, report),
        "subscription_statuses" => {
            validate_typed_restore_row::<SubscriptionStatusesRestoreRow>(row, report);
        }
        "tags" => validate_typed_restore_row::<TagsRestoreRow>(row, report),
        "target_kinds" => validate_typed_restore_row::<TargetKindsRestoreRow>(row, report),
        "user_config" => validate_typed_restore_row::<UserConfigRestoreRow>(row, report),
        "users" => validate_typed_restore_row::<UsersRestoreRow>(row, report),
        _ => {}
    }
}

/// Validates the singleton instance identity before a restore clears the target.
///
/// A current-schema backup must carry one canonical UUID. Pre-identity backups
/// cannot reach this point because the schema-version gate rejects them before
/// import; accepting them needs a deliberately versioned compatibility path.
pub(crate) fn validate_instance_identity_backup(
    source_path: &std::path::Path,
    manifest: &BackupManifest,
) -> Result<(), crate::backup::BackupError> {
    if !manifest
        .tables
        .iter()
        .any(|table| table == "instance_identity")
    {
        return Err(crate::backup::BackupError::InvalidBackup(
            "current-schema backup is missing instance_identity".to_owned(),
        ));
    }
    let rows = format::read_table_rows(source_path, "instance_identity")?;
    let [row] = rows.as_slice() else {
        return Err(crate::backup::BackupError::InvalidBackup(
            "instance_identity must contain exactly one row".to_owned(),
        ));
    };
    let Some(value) = row
        .get("instance_id")
        .and_then(format::json_value_as_restore_text)
    else {
        return Err(crate::backup::BackupError::InvalidBackup(
            "instance_identity.instance_id must be a string".to_owned(),
        ));
    };
    value.parse::<InstanceId>().map_err(|_| {
        crate::backup::BackupError::InvalidBackup(
            "instance_identity.instance_id must be a canonical UUID".to_owned(),
        )
    })?;
    Ok(())
}

trait RestoreTableRow: Sized {
    fn from_restore(row: &RestoreRowMap) -> Self;

    fn validate(&self, report: &mut RestoreValidationReport);
}

fn validate_typed_restore_row<R>(row: &RestoreRowMap, report: &mut RestoreValidationReport)
where
    R: RestoreTableRow,
{
    R::from_restore(row).validate(report);
}

struct RestoreColumn<T> {
    value: Option<Result<T, String>>,
}

impl<T> RestoreColumn<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    fn read(row: &RestoreRowMap, column: &str) -> Self {
        Self {
            value: restore_text(row, column)
                .map(|raw| raw.as_str().parse::<T>().map_err(|error| error.to_string())),
        }
    }

    fn report(
        &self,
        report: &mut RestoreValidationReport,
        table: &str,
        column: &str,
        value_class: &str,
    ) {
        if let Some(Err(reason)) = &self.value {
            push_issue(report, table, column, value_class, reason.clone());
        }
    }

    fn as_valid(&self) -> Option<&T> {
        match &self.value {
            Some(Ok(value)) => Some(value),
            Some(Err(_)) | None => None,
        }
    }
}

macro_rules! typed_restore_row {
    (
        $name:ident,
        $table:literal {
            $($field:ident: $ty:ty => ($column:literal, $value_class:literal)),+ $(,)?
        }
    ) => {
        struct $name {
            $($field: RestoreColumn<$ty>,)+
        }

        impl RestoreTableRow for $name {
            fn from_restore(row: &RestoreRowMap) -> Self {
                Self {
                    $($field: RestoreColumn::read(row, $column),)+
                }
            }

            fn validate(&self, report: &mut RestoreValidationReport) {
                $(self.$field.report(report, $table, $column, $value_class);)+
            }
        }
    };
}

typed_restore_row!(AudiencesRestoreRow, "audiences" {
    name: AudienceName => ("name", "audience name"),
});

typed_restore_row!(ChannelsRestoreRow, "channels" {
    name: Channel => ("name", "channel"),
});

typed_restore_row!(EmailVerificationsRestoreRow, "email_verifications" {
    token_hash: TokenHash => ("token_hash", "token hash"),
    email: Email => ("email", "email"),
});

typed_restore_row!(FeedEventsRestoreRow, "feed_events" {
    feed_url: FeedPath => ("feed_url", "feed path"),
    status: FeedEventStatus => ("status", "feed event status"),
});

struct IdempotencyKeysRestoreRow {
    key: Option<RestoreText>,
}

impl RestoreTableRow for IdempotencyKeysRestoreRow {
    fn from_restore(row: &RestoreRowMap) -> Self {
        Self {
            key: restore_text(row, "key"),
        }
    }

    fn validate(&self, report: &mut RestoreValidationReport) {
        let Some(raw_key) = &self.key else {
            return;
        };
        let key = match raw_key.as_str().parse::<IdempotencyKey>() {
            Ok(key) => key,
            Err(error) => {
                push_issue(
                    report,
                    "idempotency_keys",
                    "key",
                    "idempotency key",
                    error.to_string(),
                );
                return;
            }
        };
        if key.as_ref() != raw_key.as_str() {
            push_issue(
                report,
                "idempotency_keys",
                "key",
                "idempotency key",
                "idempotency key must be canonical (without surrounding whitespace)",
            );
        }
    }
}

typed_restore_row!(InvitesRestoreRow, "invites" {
    code: InviteCode => ("code", "invite code"),
});

typed_restore_row!(MediaRestoreRow, "media" {
    sha256: ContentHash => ("sha256", "content hash"),
    filename: Filename => ("filename", "filename"),
    source: MediaSource => ("source", "media source"),
    content_type: ContentType => ("content_type", "content type"),
    size_bytes: ByteSize => ("size_bytes", "byte size"),
    source_url: MediaSourceUrl => ("source_url", "media source URL"),
});

typed_restore_row!(PasswordResetsRestoreRow, "password_resets" {
    token_hash: TokenHash => ("token_hash", "token hash"),
});

typed_restore_row!(PostMediaRestoreRow, "post_media" {
    source: MediaSource => ("source", "media source"),
    sha256: ContentHash => ("sha256", "content hash"),
    filename: Filename => ("filename", "filename"),
    reference_kind: MediaReferenceKind => ("reference_kind", "media reference kind"),
    reference_form: MediaReferenceForm => ("reference_form", "media reference form"),
});

typed_restore_row!(PostRevisionsRestoreRow, "post_revisions" {
    title: PostTitle => ("title", "post title"),
    slug: Slug => ("slug", "slug"),
    body: PostBody => ("body", "post body"),
    format: PostFormat => ("format", "post format"),
    summary: PostSummary => ("summary", "post summary"),
});

typed_restore_row!(PostRevisionAudiencesRestoreRow, "post_revision_audiences" {
    target_kind: TargetKind => ("target_kind", "audience target kind"),
});

typed_restore_row!(PostRevisionTagsRestoreRow, "post_revision_tags" {
    tag_slug: Tag => ("tag_slug", "tag"),
    tag_display: TagLabel => ("tag_display", "tag label"),
});

typed_restore_row!(PostTagsRestoreRow, "post_tags" {
    tag_display: TagLabel => ("tag_display", "tag label"),
});

typed_restore_row!(PostsRestoreRow, "posts" {
    title: PostTitle => ("title", "post title"),
    slug: Slug => ("slug", "slug"),
    body: PostBody => ("body", "post body"),
    format: PostFormat => ("format", "post format"),
    summary: PostSummary => ("summary", "post summary"),
});

typed_restore_row!(SessionsRestoreRow, "sessions" {
    token_hash: TokenHash => ("token_hash", "token hash"),
});

typed_restore_row!(SubscriptionStatusesRestoreRow, "subscription_statuses" {
    name: SubscriptionStatus => ("name", "subscription status"),
});

typed_restore_row!(TagsRestoreRow, "tags" {
    tag_slug: Tag => ("tag_slug", "tag"),
});

typed_restore_row!(TargetKindsRestoreRow, "target_kinds" {
    name: TargetKind => ("name", "target kind"),
});

typed_restore_row!(UsersRestoreRow, "users" {
    username: Username => ("username", "username"),
    password_hash: StoredPasswordHash => ("password_hash", "stored password hash"),
    display_name: DisplayName => ("display_name", "display name"),
    bio: Bio => ("bio", "bio"),
    email: Email => ("email", "email"),
});

struct SiteConfigRestoreRow {
    key: RestoreColumn<SiteConfigKey>,
    value: Option<RestoreText>,
}

impl RestoreTableRow for SiteConfigRestoreRow {
    fn from_restore(row: &RestoreRowMap) -> Self {
        Self {
            key: RestoreColumn::read(row, "key"),
            value: restore_text(row, "value"),
        }
    }

    fn validate(&self, report: &mut RestoreValidationReport) {
        self.key
            .report(report, "site_config", "key", "site config key");
        if let (Some(key), Some(value)) = (self.key.as_valid(), self.value.as_ref())
            && let Err(error) = key.validate(value.as_str())
        {
            push_issue(
                report,
                "site_config",
                "value",
                "site config value",
                error.to_string(),
            );
        }
    }
}

struct UserConfigRestoreRow {
    key: RestoreColumn<UserConfigKey>,
    value: Option<RestoreText>,
}

impl RestoreTableRow for UserConfigRestoreRow {
    fn from_restore(row: &RestoreRowMap) -> Self {
        Self {
            key: RestoreColumn::read(row, "key"),
            value: restore_text(row, "value"),
        }
    }

    fn validate(&self, report: &mut RestoreValidationReport) {
        self.key
            .report(report, "user_config", "key", "user config key");
        if let (Some(key), Some(value)) = (self.key.as_valid(), self.value.as_ref())
            && let Err(error) = key.validate(value.as_str())
        {
            push_issue(
                report,
                "user_config",
                "value",
                "user config value",
                error.to_string(),
            );
        }
    }
}

fn restore_text(row: &RestoreRowMap, column: &str) -> Option<RestoreText> {
    format::json_value_as_restore_text(row.get(column)?).map(RestoreText::new)
}
fn push_issue(
    report: &mut RestoreValidationReport,
    table: &str,
    column: &str,
    value_class: &str,
    reason: impl Into<String>,
) {
    report.push(RestoreValidationIssue {
        table: table.to_owned(),
        column: column.to_owned(),
        value_class: value_class.to_owned(),
        reason: reason.into(),
    });
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreCoverageMode {
    Validated { bad_value: RestoreBadValue },
    PrimitiveRestore { rationale: &'static str },
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreBadValue {
    Text(&'static str),
    Number(i64),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RestoreColumnCoverage {
    pub(crate) table: &'static str,
    pub(crate) column: &'static str,
    pub(crate) mode: RestoreCoverageMode,
}

#[cfg(test)]
pub(crate) const RESTORE_COLUMN_COVERAGE: &[RestoreColumnCoverage] = &[
    covered("audiences", "name", RestoreBadValue::Text("")),
    covered("channels", "name", RestoreBadValue::Text("remote")),
    covered(
        "email_verifications",
        "token_hash",
        RestoreBadValue::Text("!"),
    ),
    covered(
        "email_verifications",
        "email",
        RestoreBadValue::Text("not-email"),
    ),
    covered(
        "feed_events",
        "feed_url",
        RestoreBadValue::Text("not a feed"),
    ),
    covered("feed_events", "status", RestoreBadValue::Text("sideways")),
    covered("idempotency_keys", "key", RestoreBadValue::Text("")),
    covered("invites", "code", RestoreBadValue::Text("!")),
    covered("media", "sha256", RestoreBadValue::Text("bad")),
    covered("media", "filename", RestoreBadValue::Text("my photo.jpg")),
    covered("media", "source", RestoreBadValue::Text("sideways")),
    covered("media", "content_type", RestoreBadValue::Text("notatype")),
    covered("media", "size_bytes", RestoreBadValue::Number(-1)),
    covered("media", "source_url", RestoreBadValue::Text("not a url")),
    covered("password_resets", "token_hash", RestoreBadValue::Text("!")),
    covered("post_media", "source", RestoreBadValue::Text("sideways")),
    covered("post_media", "sha256", RestoreBadValue::Text("bad")),
    covered(
        "post_media",
        "filename",
        RestoreBadValue::Text("my photo.jpg"),
    ),
    covered(
        "post_media",
        "reference_kind",
        RestoreBadValue::Text("not-a-reference-kind"),
    ),
    primitive(
        "post_media",
        "reference_form",
        "reference form is exact persisted evidence validated by the media parser on write",
    ),
    covered(
        "post_revisions",
        "format",
        RestoreBadValue::Text("sideways"),
    ),
    covered("post_revisions", "title", RestoreBadValue::Text("")),
    covered("post_revisions", "body", RestoreBadValue::Text("   ")),
    covered("post_revisions", "summary", RestoreBadValue::Text("")),
    covered(
        "post_revision_tags",
        "tag_display",
        RestoreBadValue::Text(""),
    ),
    covered(
        "post_revision_tags",
        "tag_slug",
        RestoreBadValue::Text("Bad Tag!"),
    ),
    covered(
        "post_revision_audiences",
        "target_kind",
        RestoreBadValue::Text("private"),
    ),
    covered("post_tags", "tag_display", RestoreBadValue::Text("")),
    covered("posts", "title", RestoreBadValue::Text("")),
    covered("posts", "slug", RestoreBadValue::Text("!")),
    covered("posts", "body", RestoreBadValue::Text("   ")),
    covered("posts", "format", RestoreBadValue::Text("sideways")),
    covered("posts", "summary", RestoreBadValue::Text("")),
    covered("sessions", "token_hash", RestoreBadValue::Text("!")),
    covered("site_config", "key", RestoreBadValue::Text("bad.key")),
    covered("site_config", "value", RestoreBadValue::Text("0")),
    covered(
        "subscription_statuses",
        "name",
        RestoreBadValue::Text("inactive"),
    ),
    covered("tags", "tag_slug", RestoreBadValue::Text("Bad Tag!")),
    covered("target_kinds", "name", RestoreBadValue::Text("private")),
    covered("user_config", "key", RestoreBadValue::Text("bad.key")),
    covered("user_config", "value", RestoreBadValue::Text("sideways")),
    primitive(
        "instance_identity",
        "instance_id",
        "singleton identity is validated before restore clears the target",
    ),
    covered("users", "username", RestoreBadValue::Text("Bad User")),
    covered("users", "password_hash", RestoreBadValue::Text("")),
    covered("users", "display_name", RestoreBadValue::Text("")),
    covered("users", "bio", RestoreBadValue::Text("\n")),
    covered("users", "email", RestoreBadValue::Text("not-email")),
    primitive(
        "audience_members",
        "audience_id",
        "foreign-key id: schema validation preserves the value",
    ),
    primitive(
        "audience_members",
        "subscription_id",
        "foreign-key id: schema validation preserves the value",
    ),
    primitive(
        "audience_members",
        "author_user_id",
        "foreign-key id: schema validation preserves the value",
    ),
    primitive(
        "post_revisions",
        "rendered_html",
        "trusted rendered HTML has no restore-time parser",
    ),
    primitive(
        "posts",
        "rendered_html",
        "trusted rendered HTML has no restore-time parser",
    ),
    primitive(
        "sessions",
        "label",
        "session labels are repaired through SessionLabel::from_lossy on read",
    ),
    primitive(
        "subscriptions",
        "subscriber_ref",
        "SubscriberRef is infallible and interpreted with channel_id",
    ),
];

// cov:ignore-start - test-only inventory constructors are exercised by their generated entries.
#[cfg(test)]
const fn covered(
    table: &'static str,
    column: &'static str,
    bad_value: RestoreBadValue,
) -> RestoreColumnCoverage {
    RestoreColumnCoverage {
        table,
        column,
        mode: RestoreCoverageMode::Validated { bad_value },
    }
}

#[cfg(test)]
const fn primitive(
    table: &'static str,
    column: &'static str,
    rationale: &'static str,
) -> RestoreColumnCoverage {
    RestoreColumnCoverage {
        table,
        column,
        mode: RestoreCoverageMode::PrimitiveRestore { rationale },
    }
}
// cov:ignore-stop

#[cfg(test)]
const BACKED_UP_DOMAIN_COLUMNS: &[(&str, &str)] = &[
    ("instance_identity", "instance_id"),
    ("audience_members", "audience_id"),
    ("audience_members", "author_user_id"),
    ("audience_members", "subscription_id"),
    ("audiences", "name"),
    ("channels", "name"),
    ("email_verifications", "email"),
    ("email_verifications", "token_hash"),
    ("feed_events", "feed_url"),
    ("feed_events", "status"),
    ("idempotency_keys", "key"),
    ("invites", "code"),
    ("media", "content_type"),
    ("media", "filename"),
    ("media", "sha256"),
    ("media", "size_bytes"),
    ("media", "source"),
    ("media", "source_url"),
    ("password_resets", "token_hash"),
    ("post_media", "filename"),
    ("post_media", "sha256"),
    ("post_media", "source"),
    ("post_media", "reference_kind"),
    ("post_media", "reference_form"),
    ("post_revisions", "body"),
    ("post_revisions", "format"),
    ("post_revisions", "rendered_html"),
    ("post_revisions", "title"),
    ("post_revisions", "summary"),
    ("post_revision_tags", "tag_display"),
    ("post_revision_tags", "tag_slug"),
    ("post_revision_audiences", "target_kind"),
    ("post_tags", "tag_display"),
    ("posts", "body"),
    ("posts", "format"),
    ("posts", "rendered_html"),
    ("posts", "slug"),
    ("posts", "summary"),
    ("posts", "title"),
    ("sessions", "label"),
    ("sessions", "token_hash"),
    ("site_config", "key"),
    ("site_config", "value"),
    ("subscription_statuses", "name"),
    ("subscriptions", "subscriber_ref"),
    ("tags", "tag_slug"),
    ("target_kinds", "name"),
    ("user_config", "key"),
    ("user_config", "value"),
    ("users", "bio"),
    ("users", "display_name"),
    ("users", "email"),
    ("users", "password_hash"),
    ("users", "username"),
];
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{BackupManifest, BackupMode};
    use crate::backup::{CatalogColumnName, CatalogTableName};
    use common::time::UtcInstant;
    use sqlx::Row;
    use std::collections::BTreeSet;
    use tempfile::TempDir;
    // guard:low-level-db — compares the restore domain-column inventory to the live SQLite backup schema surface.
    #[tokio::test]
    async fn restore_typed_column_inventory_matches_current_backup_schema() {
        let env = crate::test_support::Backend::Sqlite.setup().await;
        let crate::test_support::CloseablePool::Sqlite(pool) = env.base.pool() else {
            unreachable!("SQLite backend yields a SQLite pool")
        };
        let backed_up_columns = backed_up_schema_columns(pool).await;
        let domain_columns = domain_column_keys();

        for domain_column in &domain_columns {
            assert!(
                backed_up_columns.contains(domain_column),
                "{domain_column} is in the restore domain-column inventory but is not in the current backed-up schema"
            );
        }

        let coverage_columns = coverage_column_keys();
        assert_eq!(
            coverage_columns, domain_columns,
            "restore validation coverage must match the backed-up domain-column inventory"
        );
    }

    #[test]
    fn restore_typed_column_inventory_is_unique() {
        let actual = coverage_column_keys();
        let unique = actual.iter().collect::<BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            actual.len(),
            "inventory must not duplicate columns"
        );
    }

    fn identity_manifest(tables: Vec<String>) -> BackupManifest {
        BackupManifest {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: 27,
            schema_checksum: "test".to_owned(),
            timestamp: UtcInstant::now(),
            mode: BackupMode::Directory,
            tables,
        }
    }

    #[test]
    fn restore_identity_validation_rejects_every_malformed_backup_shape() {
        let temp = TempDir::new().expect("make backup fixture");
        let database = temp.path().join("db");
        std::fs::create_dir(&database).expect("make backup database directory");

        let missing = identity_manifest(Vec::new());
        assert!(validate_instance_identity_backup(temp.path(), &missing).is_err());

        let manifest = identity_manifest(vec!["instance_identity".to_owned()]);
        for rows in [
            "",
            "{\"instance_id\":\"123e4567-e89b-12d3-a456-426614174000\"}\n{\"instance_id\":\"123e4567-e89b-12d3-a456-426614174000\"}\n",
            "{}\n",
            "{\"instance_id\":7}\n",
            "{\"instance_id\":\"not-a-uuid\"}\n",
        ] {
            std::fs::write(database.join("instance_identity.ndjson"), rows)
                .expect("write malformed identity rows");
            assert!(
                validate_instance_identity_backup(temp.path(), &manifest).is_err(),
                "{rows:?} must be rejected"
            );
        }
    }

    #[test]
    fn every_validated_inventory_column_reaches_the_restore_row_validator() {
        for entry in RESTORE_COLUMN_COVERAGE {
            let RestoreCoverageMode::Validated { bad_value } = entry.mode else {
                let RestoreCoverageMode::PrimitiveRestore { rationale } = entry.mode else {
                    unreachable!("coverage mode is exhaustive")
                };
                assert!(!rationale.trim().is_empty());
                continue;
            };

            let mut row = serde_json::Map::new();
            add_context_columns(entry.table, entry.column, &mut row);
            row.insert(entry.column.to_owned(), bad_value_json(bad_value));
            let mut report = RestoreValidationReport::default();

            validate_restore_row(entry.table, &row, &mut report);

            assert!(
                report
                    .issues()
                    .iter()
                    .any(|issue| issue.table == entry.table && issue.column == entry.column),
                "{}.{} was inventoried but did not report through the row validator",
                entry.table,
                entry.column
            );
        }
    }

    #[test]
    fn idempotency_key_restore_row_allows_missing_key_for_structural_validation() {
        let row = serde_json::Map::new();
        let mut report = RestoreValidationReport::default();

        validate_restore_row("idempotency_keys", &row, &mut report);

        assert!(report.is_empty(), "missing key produced {report:?}");
    }

    #[test]
    fn idempotency_key_restore_row_accepts_valid_key() {
        let mut row = serde_json::Map::new();
        row.insert("key".to_owned(), serde_json::json!("retry-key"));
        let mut report = RestoreValidationReport::default();

        validate_restore_row("idempotency_keys", &row, &mut report);

        assert!(report.is_empty(), "valid key produced {report:?}");
    }

    #[test]
    fn idempotency_key_restore_row_reports_blank_keys() {
        for key in ["", " \t\n"] {
            let mut row = serde_json::Map::new();
            row.insert("key".to_owned(), serde_json::json!(key));
            let mut report = RestoreValidationReport::default();

            validate_restore_row("idempotency_keys", &row, &mut report);

            assert_eq!(
                report.issues(),
                &[RestoreValidationIssue {
                    table: "idempotency_keys".to_owned(),
                    column: "key".to_owned(),
                    value_class: "idempotency key".to_owned(),
                    reason: "idempotency key must be non-empty".to_owned(),
                }],
                "key {key:?}"
            );
        }
    }

    #[test]
    fn idempotency_key_restore_row_reports_padded_keys() {
        let mut row = serde_json::Map::new();
        row.insert("key".to_owned(), serde_json::json!(" retry-key\t"));
        let mut report = RestoreValidationReport::default();

        validate_restore_row("idempotency_keys", &row, &mut report);

        assert_eq!(
            report.issues(),
            &[RestoreValidationIssue {
                table: "idempotency_keys".to_owned(),
                column: "key".to_owned(),
                value_class: "idempotency key".to_owned(),
                reason: "idempotency key must be canonical (without surrounding whitespace)"
                    .to_owned(),
            }]
        );
    }

    #[test]
    fn media_filename_diagnostic_names_the_noncanonical_value_class() {
        let mut row = serde_json::Map::new();
        row.insert("filename".to_owned(), serde_json::json!("my photo.jpg"));
        let mut report = RestoreValidationReport::default();

        validate_restore_row("media", &row, &mut report);

        let issue = report
            .issues()
            .iter()
            .find(|issue| issue.table == "media" && issue.column == "filename")
            .expect("media.filename issue");
        assert_eq!(issue.value_class, "filename");
        assert!(
            issue.reason.contains("canonical percent-encoded"),
            "reason should name canonicity, got {issue:?}"
        );
    }

    async fn backed_up_schema_columns(pool: &sqlx::SqlitePool) -> BTreeSet<String> {
        let names = sqlx::query_scalar::<_, CatalogTableName>(
            "SELECT name FROM sqlite_master WHERE type = 'table'",
        )
        .fetch_all(pool)
        .await
        .expect("read current SQLite schema tables")
        .into_iter()
        .map(CatalogTableName::into_inner)
        .collect::<Vec<String>>();
        let tables = super::super::backup_table_set(names);
        let mut columns = BTreeSet::new();
        for table in tables {
            let pragma = format!(
                "PRAGMA table_info({})",
                crate::sql::quote_identifier(&table)
            );
            let rows = sqlx::query(&pragma)
                .fetch_all(pool)
                .await
                .expect("read current SQLite schema columns");
            for row in rows {
                let column = row
                    .try_get::<CatalogColumnName, _>("name")
                    .expect("column name")
                    .into_inner();
                columns.insert(format!("{table}.{column}"));
            }
        }
        columns
    }

    fn domain_column_keys() -> Vec<String> {
        let mut keys = BACKED_UP_DOMAIN_COLUMNS
            .iter()
            .map(|(table, column)| format!("{table}.{column}"))
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn coverage_column_keys() -> Vec<String> {
        let mut keys = RESTORE_COLUMN_COVERAGE
            .iter()
            .map(|entry| format!("{}.{}", entry.table, entry.column))
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn add_context_columns(
        table: &str,
        column: &str,
        row: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        match (table, column) {
            ("site_config", "value") => {
                row.insert(
                    "key".to_owned(),
                    serde_json::json!("media.max_file_size_bytes"),
                );
            }
            ("user_config", "value") => {
                row.insert("key".to_owned(), serde_json::json!("posts.default_format"));
            }
            _ => {}
        }
    }

    fn bad_value_json(value: RestoreBadValue) -> serde_json::Value {
        match value {
            RestoreBadValue::Text(value) => serde_json::json!(value),
            RestoreBadValue::Number(value) => serde_json::json!(value),
        }
    }
}
