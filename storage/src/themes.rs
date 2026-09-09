//! Typed relational primitives for custom public-theme aggregates.
//!
//! This module deliberately stores compiler output as opaque validated values. Archive
//! parsing, CSS transformation, and filesystem materialization live outside storage.

use std::collections::BTreeSet;

use crate::WriteTransaction;
use crate::write_scope;
use async_trait::async_trait;
use common::{
    ids::{ThemeId, UserId},
    media::{ContentHash, Filename, MediaRef, MediaSource},
    theme::{
        PublicThemeSelection, ThemeAssetDigest, ThemeContentDigest, ThemeImageRole,
        ThemePoolRevisionDigest, ThemeRevisionDigest, ThemeSourceDigest, ThemeStylesheetDigest,
    },
};

/// The catalog that owns a custom theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeOwner {
    /// The operator-owned site catalog.
    Site,
    /// One author's private catalog.
    Author(UserId),
}

/// A catalog entry, independent from its mutable draft and immutable revisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeCatalogEntry {
    pub id: ThemeId,
    pub owner: ThemeOwner,
    pub name: String,
    pub current_revision: Option<ThemeRevisionDigest>,
}
/// Maximum length of a human-visible custom theme catalog name, in Unicode scalar values.
pub const THEME_CATALOG_NAME_MAX_LENGTH: usize = 100;

/// Why a catalog name cannot be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThemeCatalogNameError {
    #[error("theme name must not be empty")]
    Empty,
    #[error("theme name must not exceed {THEME_CATALOG_NAME_MAX_LENGTH} characters")]
    TooLong,
    #[error("theme name conflicts with a built-in theme")]
    Reserved,
}

/// Trims and validates a display name before it is written to a custom-theme catalog.
///
/// Built-in theme names remain reserved case-insensitively so a custom entry cannot
/// impersonate a fixed selection.
///
/// # Errors
///
/// Returns [`ThemeCatalogNameError`] when the trimmed name is empty, too long,
/// or matches a built-in theme label.
pub fn validate_theme_catalog_name(name: &str) -> Result<String, ThemeCatalogNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ThemeCatalogNameError::Empty);
    }
    if name.chars().count() > THEME_CATALOG_NAME_MAX_LENGTH {
        return Err(ThemeCatalogNameError::TooLong);
    }
    if ["terminal", "studio", "reader"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err(ThemeCatalogNameError::Reserved);
    }
    Ok(name.to_owned())
}

/// One validated package member retained with a mutable theme draft.
///
/// Draft reads return assets in canonical lexical path order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeDraftAsset {
    pub path: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub digest: ThemeAssetDigest,
}

/// The single mutable package draft for a Theme ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeDraft {
    pub theme_id: ThemeId,
    pub manifest: Vec<u8>,
    pub stylesheet: Vec<u8>,
    pub source_digest: ThemeSourceDigest,
    pub assets: Vec<ThemeDraftAsset>,
}

enum DraftAssetColumns {
    Empty,
    Asset(ThemeDraftAsset),
}

fn draft_asset_from_columns(
    path: Option<String>,
    mime: Option<String>,
    bytes: Option<Vec<u8>>,
    digest: Option<String>,
) -> Option<DraftAssetColumns> {
    match (path, mime, bytes, digest) {
        (Some(path), Some(mime), Some(bytes), Some(digest)) => digest.parse().ok().map(|digest| {
            DraftAssetColumns::Asset(ThemeDraftAsset {
                path,
                mime,
                bytes,
                digest,
            })
        }),
        (None, None, None, None) => Some(DraftAssetColumns::Empty),
        _ => None,
    }
}

/// One immutable published theme revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeRevision {
    pub theme_id: ThemeId,
    pub digest: ThemeRevisionDigest,
    pub stylesheet_digest: ThemeStylesheetDigest,
    pub manifest: Vec<u8>,
}
/// One immutable package asset belonging to a published revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemePackageAsset {
    pub path: String,
    pub digest: ThemeAssetDigest,
    pub mime: String,
}

/// Theme retention and serving eligibility are intentionally separate from blobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeContentEligibility {
    pub digest: ThemeContentDigest,
    pub mime: String,
    pub retained_until_unix_seconds: i64,
}

/// One validated logo/header binding.
///
/// The nullable relational representation stays at the database boundary. Every
/// value that reaches callers is one of these complete, role-valid alternatives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeRoleBinding {
    PackagedDefault {
        theme_id: ThemeId,
        role: ThemeImageRole,
    },
    ExplicitAbsent {
        theme_id: ThemeId,
        role: ThemeImageRole,
    },
    PackageAsset {
        theme_id: ThemeId,
        role: ThemeImageRole,
        package_path: String,
    },
    Media {
        theme_id: ThemeId,
        role: ThemeImageRole,
        user_id: UserId,
        media: MediaRef,
    },
    HeaderPool {
        theme_id: ThemeId,
        pool_revision: ThemePoolRevisionDigest,
        shuffle_seed: [u8; 32],
    },
}

impl ThemeRoleBinding {
    #[must_use]
    pub fn theme_id(&self) -> ThemeId {
        match self {
            Self::PackagedDefault { theme_id, .. }
            | Self::ExplicitAbsent { theme_id, .. }
            | Self::PackageAsset { theme_id, .. }
            | Self::Media { theme_id, .. }
            | Self::HeaderPool { theme_id, .. } => *theme_id,
        }
    }

    #[must_use]
    pub fn role(&self) -> ThemeImageRole {
        match self {
            Self::PackagedDefault { role, .. }
            | Self::ExplicitAbsent { role, .. }
            | Self::PackageAsset { role, .. }
            | Self::Media { role, .. } => *role,
            Self::HeaderPool { .. } => ThemeImageRole::Header,
        }
    }

    #[must_use]
    pub fn is_explicit_absent(&self) -> bool {
        matches!(self, Self::ExplicitAbsent { .. })
    }
}

/// A canonical member of an explicit header pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeHeaderPoolEntry {
    pub ordinal: i64,
    pub package_path: Option<String>,
    pub media_user_id: Option<UserId>,
    pub media_source: Option<String>,
    pub media_digest: Option<ThemeContentDigest>,
    pub media_filename: Option<String>,
}

/// One exact Media reference persisted by a theme binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeMediaReference {
    pub user_id: UserId,
    pub media: MediaRef,
}

/// Counters used by the centralized owner admission lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeOwnerQuota {
    pub active_themes: i64,
    pub retained_revisions: i64,
    pub logical_bytes: i64,
}

/// A rejected draft replacement that leaves the persisted draft untouched.
#[derive(Debug, thiserror::Error)]
pub enum ReplaceDraftError {
    #[error("theme draft does not exist in this catalog")]
    OwnerNotFound,
    #[error("theme draft replacement exceeds quota")]
    QuotaExceeded,
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

/// Counters used by the site-wide admission lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeSiteQuota {
    pub retained_revisions: i64,
    pub physical_bytes: i64,
}

/// Limits applied while the storage-owned admission lock is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeQuotaLimits {
    pub active_themes: i64,
    pub retained_revisions: i64,
    pub logical_bytes: i64,
    pub site_retained_revisions: i64,
    pub site_physical_bytes: i64,
}
/// Conservative production ceiling for all mutable and published theme content.
///
/// Composition roots may inject a stricter policy, but public endpoints must
/// never turn quota accounting into an unbounded allocation policy.
pub const PRODUCTION_THEME_QUOTA_LIMITS: ThemeQuotaLimits = ThemeQuotaLimits {
    active_themes: 64,
    retained_revisions: 256,
    logical_bytes: 512 * 1024 * 1024,
    site_retained_revisions: 4_096,
    site_physical_bytes: 8 * 1024 * 1024 * 1024,
};

impl ThemeQuotaLimits {
    #[must_use]
    pub const fn production() -> Self {
        PRODUCTION_THEME_QUOTA_LIMITS
    }
}

/// A retained content identity and its durable owner/site byte charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeContentCharge {
    pub digest: ThemeContentDigest,
    pub logical_bytes: i64,
    pub physical_bytes: i64,
}

/// All inputs admitted atomically for a published immutable theme revision.
#[derive(Clone, Copy, Debug)]
pub struct ThemePublicationAdmission<'a> {
    pub owner: ThemeOwner,
    pub limits: ThemeQuotaLimits,
    pub revision: &'a ThemeRevision,
    pub assets: &'a [ThemePackageAsset],
    pub eligibilities: &'a [ThemeContentEligibility],
    pub charges: &'a [ThemeContentCharge],
}

/// Async relational operations used by publication, selection, and Media-role services.
/// Every mutation joins the caller-owned transaction capability.
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait ThemeStorage: Send + Sync {
    /// Creates a catalog entry after admitting its active-theme quota in this
    /// transaction. Publication never changes that catalog count.
    async fn create_theme(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        name: &str,
        draft: &ThemeDraft,
        limits: ThemeQuotaLimits,
    ) -> Result<ThemeId, sqlx::Error>;
    async fn list_themes(&self, owner: ThemeOwner) -> Result<Vec<ThemeCatalogEntry>, sqlx::Error>;
    async fn get_draft(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
    ) -> Result<Option<ThemeDraft>, sqlx::Error>;
    /// Replaces a draft only after atomically admitting its new byte delta.
    async fn replace_draft(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        draft: &ThemeDraft,
        limits: ThemeQuotaLimits,
    ) -> Result<(), ReplaceDraftError>;
    async fn rename_theme(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        theme_id: ThemeId,
        name: &str,
    ) -> Result<(), sqlx::Error>;
    async fn record_revision(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        revision: &ThemeRevision,
        assets: &[ThemePackageAsset],
    ) -> Result<(), sqlx::Error>;
    async fn list_revisions(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
    ) -> Result<Vec<ThemeRevision>, sqlx::Error>;
    /// Lists immutable package assets for one owned published revision.
    async fn revision_assets(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
        revision: &ThemeRevisionDigest,
    ) -> Result<Vec<ThemePackageAsset>, sqlx::Error>;
    async fn upsert_content_eligibility(
        &self,
        transaction: &mut WriteTransaction,
        eligibility: &ThemeContentEligibility,
    ) -> Result<(), sqlx::Error>;
    async fn content_eligibility(
        &self,
        digest: &ThemeContentDigest,
    ) -> Result<Option<ThemeContentEligibility>, sqlx::Error>;
    /// Lists every digest that is eligible for immutable serving or retention.
    async fn list_content_eligibility(&self) -> Result<Vec<ThemeContentEligibility>, sqlx::Error>;
    async fn set_selection(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        selection: Option<PublicThemeSelection>,
    ) -> Result<(), sqlx::Error>;
    async fn selection(
        &self,
        owner: ThemeOwner,
    ) -> Result<Option<PublicThemeSelection>, sqlx::Error>;
    /// Removes a catalog and all its revisions while retaining their content
    /// accounting through the supplied deadline.
    async fn remove_theme(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        theme_id: ThemeId,
        retained_until_unix_seconds: i64,
    ) -> Result<(), sqlx::Error>;
    async fn replace_role_binding(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        binding: &ThemeRoleBinding,
    ) -> Result<(), sqlx::Error>;
    async fn role_binding(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
        role: ThemeImageRole,
    ) -> Result<Option<ThemeRoleBinding>, sqlx::Error>;
    async fn replace_header_pool(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        theme_id: ThemeId,
        entries: &[ThemeHeaderPoolEntry],
    ) -> Result<(), sqlx::Error>;
    async fn header_pool(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
    ) -> Result<Vec<ThemeHeaderPoolEntry>, sqlx::Error>;
    /// Locks a theme aggregate in the caller's transaction and returns its
    /// complete, canonically ordered Media reference set.
    async fn locked_media_references(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        theme_id: ThemeId,
    ) -> Result<Vec<ThemeMediaReference>, sqlx::Error>;
    /// Changes only the seed of an existing header pool binding.
    async fn shuffle_header_pool(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        theme_id: ThemeId,
        shuffle_seed: [u8; 32],
    ) -> Result<(), sqlx::Error>;
    async fn owner_quota(&self, owner: ThemeOwner) -> Result<Option<ThemeOwnerQuota>, sqlx::Error>;
    async fn site_quota(&self) -> Result<ThemeSiteQuota, sqlx::Error>;
    /// Atomically admits one active catalog entry under the centralized quota lock.
    async fn admit_theme(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        limits: ThemeQuotaLimits,
    ) -> Result<(), sqlx::Error>;
    /// Atomically admits a new immutable revision and all of its serving content.
    ///
    /// The implementation locks the site quota, the owner quota, and content
    /// rows in digest order before it observes the current revision. Republishing
    /// that current digest is deliberately a no-op.
    async fn admit_publication<'a>(
        &self,
        transaction: &mut WriteTransaction,
        admission: ThemePublicationAdmission<'a>,
    ) -> Result<(), sqlx::Error>;
    /// Attaches sorted retained content identities and admits one revision.
    async fn attach_revision_content(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        limits: ThemeQuotaLimits,
        charges: &[ThemeContentCharge],
    ) -> Result<(), sqlx::Error>;
    /// Drops live revision references while preserving owner charges through retention.
    async fn detach_revision_content(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        charges: &[ThemeContentCharge],
        retained_until_unix_seconds: i64,
    ) -> Result<(), sqlx::Error>;
    /// Collects one elapsed, unreferenced content charge and its physical charge.
    async fn collect_retained_content(
        &self,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        digest: &ThemeContentDigest,
        now_unix_seconds: i64,
    ) -> Result<(), sqlx::Error>;
    /// Lists expired owner charges with no remaining live references.
    async fn expired_retained_content(
        &self,
        now_unix_seconds: i64,
    ) -> Result<Vec<(ThemeOwner, ThemeContentDigest)>, sqlx::Error>;
}

/// Concrete storage handle; its backend implementations deliberately remain separate.
pub struct ThemeStore<DB: sqlx::Database> {
    pool: sqlx::Pool<DB>,
}

impl<DB: sqlx::Database> ThemeStore<DB> {
    #[must_use]
    pub fn new(pool: sqlx::Pool<DB>) -> Self {
        Self { pool }
    }
}

macro_rules! impl_theme_storage {
    ($db:ty, $conn:path) => {
        #[async_trait]
        impl ThemeStorage for ThemeStore<$db> {
            async fn create_theme(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, name: &str, draft: &ThemeDraft, limits: ThemeQuotaLimits) -> Result<ThemeId, sqlx::Error> {
                let name = validate_theme_catalog_name(name)
                    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
                let connection = $conn(transaction)?;
                let owner_key = catalog_owner_key(owner);
                let bytes = draft_storage_bytes(draft)?;
                // A site lock serializes byte admission across both backends before
                // either mutable-draft or published-content counters change.
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                let site: (i64,) = sqlx::query_as("SELECT physical_bytes FROM theme_site_quota WHERE singleton = 1").fetch_one(&mut *connection).await?;
                let owner_quota: Option<(i64, i64)> = sqlx::query_as("SELECT active_themes, logical_bytes FROM theme_owner_quotas WHERE catalog_owner_key = $1").bind(&owner_key).fetch_optional(&mut *connection).await?;
                let (active, logical) = owner_quota.unwrap_or((0, 0));
                let source: Option<(i64,)> = sqlx::query_as("SELECT live_references FROM theme_draft_content_charges WHERE source_digest = $1").bind(draft.source_digest.as_ref()).fetch_optional(&mut *connection).await?;
                if active >= limits.active_themes || logical > limits.logical_bytes - bytes || (source.is_none() && site.0 > limits.site_physical_bytes - bytes) { return Err(sqlx::Error::RowNotFound); }
                if source.is_none() { sqlx::query("UPDATE theme_site_quota SET physical_bytes = physical_bytes + $1 WHERE singleton = 1").bind(bytes).execute(&mut *connection).await?; }
                sqlx::query("INSERT INTO theme_owner_quotas (catalog_owner_key, active_themes, logical_bytes) VALUES ($1, 1, $2) ON CONFLICT (catalog_owner_key) DO UPDATE SET active_themes = theme_owner_quotas.active_themes + 1, logical_bytes = theme_owner_quotas.logical_bytes + $2").bind(&owner_key).bind(bytes).execute(&mut *connection).await?;
                sqlx::query("INSERT INTO theme_draft_content_charges (source_digest, physical_bytes, live_references) VALUES ($1, $2, 1) ON CONFLICT (source_digest) DO UPDATE SET live_references = theme_draft_content_charges.live_references + 1").bind(draft.source_digest.as_ref()).bind(bytes).execute(&mut *connection).await?;
                let name_key = canonical_theme_name_key(&name);
                let (id,): (i64,) = sqlx::query_as("INSERT INTO themes (catalog_owner_key, name, name_key, current_revision_digest) VALUES ($1, $2, $3, NULL) RETURNING id").bind(&owner_key).bind(&name).bind(name_key).fetch_one(&mut *connection).await?;
                sqlx::query("INSERT INTO theme_drafts (theme_id, manifest, stylesheet, source_digest) VALUES ($1, $2, $3, $4)").bind(id).bind(&draft.manifest).bind(&draft.stylesheet).bind(draft.source_digest.as_ref()).execute(&mut *connection).await?;
                sqlx::query("INSERT INTO theme_draft_charges (theme_id, source_digest, logical_bytes, physical_bytes) VALUES ($1, $2, $3, $3)").bind(id).bind(draft.source_digest.as_ref()).bind(bytes).execute(&mut *connection).await?;
                for asset in &draft.assets { sqlx::query("INSERT INTO theme_draft_assets (theme_id, path, mime, bytes, digest) VALUES ($1, $2, $3, $4, $5)").bind(id).bind(&asset.path).bind(&asset.mime).bind(&asset.bytes).bind(asset.digest.as_ref()).execute(&mut *connection).await?; }
                Ok(ThemeId::from(id))
            }
            async fn list_themes(&self, owner: ThemeOwner) -> Result<Vec<ThemeCatalogEntry>, sqlx::Error> {
                let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as("SELECT id, name, current_revision_digest FROM themes WHERE catalog_owner_key = $1 ORDER BY name_key, id")
                    .bind(catalog_owner_key(owner)).fetch_all(&self.pool).await?;
                Ok(rows.into_iter().filter_map(|(id, name, digest)| Some(ThemeCatalogEntry { id: ThemeId::from(id), owner, name, current_revision: digest.and_then(|value| value.parse().ok()) })).collect())
            }
            async fn get_draft(&self, owner: ThemeOwner, theme_id: ThemeId) -> Result<Option<ThemeDraft>, sqlx::Error> {
                let rows: Vec<(i64, Vec<u8>, Vec<u8>, String, Option<String>, Option<String>, Option<Vec<u8>>, Option<String>)> = sqlx::query_as("SELECT d.theme_id, d.manifest, d.stylesheet, d.source_digest, asset.path, asset.mime, asset.bytes, asset.digest FROM theme_drafts d JOIN themes t ON t.id = d.theme_id LEFT JOIN theme_draft_assets asset ON asset.theme_id = d.theme_id WHERE d.theme_id = $1 AND t.catalog_owner_key = $2 ORDER BY asset.path")
                    .bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).fetch_all(&self.pool).await?;
                let mut rows = rows.into_iter();
                let Some((id, manifest, stylesheet, source_digest, path, mime, bytes, digest)) = rows.next() else {
                    return Ok(None);
                };
                let Ok(source_digest) = source_digest.parse() else {
                    return Ok(None);
                };
                let mut assets = Vec::new();
                let Some(asset) = draft_asset_from_columns(path, mime, bytes, digest) else {
                    return Ok(None);
                };
                if let DraftAssetColumns::Asset(asset) = asset {
                    assets.push(asset);
                }
                for (_, _, _, _, path, mime, bytes, digest) in rows {
                    let Some(asset) = draft_asset_from_columns(path, mime, bytes, digest) else {
                        return Ok(None);
                    };
                    let DraftAssetColumns::Asset(asset) = asset else {
                        return Ok(None);
                    };
                    assets.push(asset);
                }
                assets.sort_by(|left, right| left.path.cmp(&right.path));
                Ok(Some(ThemeDraft { theme_id: ThemeId::from(id), manifest, stylesheet, source_digest, assets }))
            }
            async fn replace_draft(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, draft: &ThemeDraft, limits: ThemeQuotaLimits) -> Result<(), ReplaceDraftError> {
                let connection = $conn(transaction)?;
                let owner_key = catalog_owner_key(owner);
                let new_bytes = draft_storage_bytes(draft)?;
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&owner_key).execute(&mut *connection).await?;
                let old: Option<(String, i64)> = sqlx::query_as("SELECT charge.source_digest, charge.logical_bytes FROM theme_draft_charges charge JOIN themes theme ON theme.id = charge.theme_id WHERE charge.theme_id = $1 AND theme.catalog_owner_key = $2").bind(i64::from(draft.theme_id)).bind(&owner_key).fetch_optional(&mut *connection).await?;
                let Some((old_digest, old_bytes)) = old else { return Err(ReplaceDraftError::OwnerNotFound); };
                let (logical,): (i64,) = sqlx::query_as("SELECT logical_bytes FROM theme_owner_quotas WHERE catalog_owner_key = $1").bind(&owner_key).fetch_one(&mut *connection).await?;
                let (site_bytes,): (i64,) = sqlx::query_as("SELECT physical_bytes FROM theme_site_quota WHERE singleton = 1").fetch_one(&mut *connection).await?;
                let old_refs: (i64,) = sqlx::query_as("SELECT live_references FROM theme_draft_content_charges WHERE source_digest = $1").bind(&old_digest).fetch_one(&mut *connection).await?;
                let new_refs: Option<(i64,)> = if old_digest == draft.source_digest.as_ref() { Some(old_refs) } else { sqlx::query_as("SELECT live_references FROM theme_draft_content_charges WHERE source_digest = $1").bind(draft.source_digest.as_ref()).fetch_optional(&mut *connection).await? };
                let logical_after = logical.checked_sub(old_bytes).and_then(|value| value.checked_add(new_bytes)).ok_or(ReplaceDraftError::QuotaExceeded)?;
                let physical_after = site_bytes.checked_sub(if old_digest != draft.source_digest.as_ref() && old_refs.0 == 1 { old_bytes } else { 0 }).and_then(|value| value.checked_add(if old_digest != draft.source_digest.as_ref() && new_refs.is_none() { new_bytes } else { 0 })).ok_or(ReplaceDraftError::QuotaExceeded)?;
                if logical_after > limits.logical_bytes || physical_after > limits.site_physical_bytes { return Err(ReplaceDraftError::QuotaExceeded); }
                sqlx::query("UPDATE theme_owner_quotas SET logical_bytes = $1 WHERE catalog_owner_key = $2").bind(logical_after).bind(&owner_key).execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_site_quota SET physical_bytes = $1 WHERE singleton = 1").bind(physical_after).execute(&mut *connection).await?;
                if old_digest != draft.source_digest.as_ref() {
                    if old_refs.0 == 1 { sqlx::query("DELETE FROM theme_draft_content_charges WHERE source_digest = $1").bind(&old_digest).execute(&mut *connection).await?; } else { sqlx::query("UPDATE theme_draft_content_charges SET live_references = live_references - 1 WHERE source_digest = $1 AND live_references > 1").bind(&old_digest).execute(&mut *connection).await?; }
                    sqlx::query("INSERT INTO theme_draft_content_charges (source_digest, physical_bytes, live_references) VALUES ($1, $2, 1) ON CONFLICT (source_digest) DO UPDATE SET live_references = theme_draft_content_charges.live_references + 1").bind(draft.source_digest.as_ref()).bind(new_bytes).execute(&mut *connection).await?;
                }
                let result = sqlx::query("UPDATE theme_drafts SET manifest = $1, stylesheet = $2, source_digest = $3 WHERE theme_id = $4").bind(&draft.manifest).bind(&draft.stylesheet).bind(draft.source_digest.as_ref()).bind(i64::from(draft.theme_id)).execute(&mut *connection).await?;
                if result.rows_affected() != 1 { return Err(ReplaceDraftError::OwnerNotFound); }
                sqlx::query("UPDATE theme_draft_charges SET source_digest = $1, logical_bytes = $2, physical_bytes = $2 WHERE theme_id = $3").bind(draft.source_digest.as_ref()).bind(new_bytes).bind(i64::from(draft.theme_id)).execute(&mut *connection).await?;
                sqlx::query("DELETE FROM theme_draft_assets WHERE theme_id = $1").bind(i64::from(draft.theme_id)).execute(&mut *connection).await?;
                for asset in &draft.assets { sqlx::query("INSERT INTO theme_draft_assets (theme_id, path, mime, bytes, digest) VALUES ($1, $2, $3, $4, $5)").bind(i64::from(draft.theme_id)).bind(&asset.path).bind(&asset.mime).bind(&asset.bytes).bind(asset.digest.as_ref()).execute(&mut *connection).await?; }
                Ok(())
            }
            async fn rename_theme(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, theme_id: ThemeId, name: &str) -> Result<(), sqlx::Error> {
                let name = validate_theme_catalog_name(name)
                    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
                let name_key = canonical_theme_name_key(&name);
                let connection = $conn(transaction)?;
                let result = sqlx::query("UPDATE themes SET name = $1, name_key = $2 WHERE id = $3 AND catalog_owner_key = $4").bind(&name).bind(name_key).bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).execute(&mut *connection).await?;
                if result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                Ok(())
            }
            async fn record_revision(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, revision: &ThemeRevision, assets: &[ThemePackageAsset]) -> Result<(), sqlx::Error> {
                let connection = $conn(transaction)?;
                let owned: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND catalog_owner_key = $2)")
                    .bind(i64::from(revision.theme_id)).bind(catalog_owner_key(owner)).fetch_one(&mut *connection).await?;
                if !owned.0 { return Err(sqlx::Error::RowNotFound); }
                sqlx::query("INSERT INTO theme_revisions (theme_id, digest, stylesheet_digest, manifest) VALUES ($1, $2, $3, $4)")
                    .bind(i64::from(revision.theme_id)).bind(revision.digest.as_ref()).bind(revision.stylesheet_digest.as_ref()).bind(&revision.manifest).execute(&mut *connection).await?;
                for asset in assets {
                    sqlx::query("INSERT INTO theme_revision_assets (theme_id, revision_digest, path, digest, mime) VALUES ($1, $2, $3, $4, $5)")
                        .bind(i64::from(revision.theme_id)).bind(revision.digest.as_ref()).bind(&asset.path).bind(asset.digest.as_ref()).bind(&asset.mime).execute(&mut *connection).await?;
                }
                sqlx::query("UPDATE themes SET current_revision_digest = $1 WHERE id = $2").bind(revision.digest.as_ref()).bind(i64::from(revision.theme_id)).execute(&mut *connection).await?;
                Ok(())
            }
            async fn list_revisions(&self, owner: ThemeOwner, theme_id: ThemeId) -> Result<Vec<ThemeRevision>, sqlx::Error> {
                let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as("SELECT r.digest, r.stylesheet_digest, r.manifest FROM theme_revisions r JOIN themes t ON t.id = r.theme_id WHERE r.theme_id = $1 AND t.catalog_owner_key = $2 ORDER BY r.id DESC").bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).fetch_all(&self.pool).await?;
                Ok(rows.into_iter().filter_map(|(digest, stylesheet_digest, manifest)| Some(ThemeRevision { theme_id, digest: digest.parse().ok()?, stylesheet_digest: stylesheet_digest.parse().ok()?, manifest })).collect())
            }
            async fn revision_assets(&self, owner: ThemeOwner, theme_id: ThemeId, revision: &ThemeRevisionDigest) -> Result<Vec<ThemePackageAsset>, sqlx::Error> {
                let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT asset.path, asset.digest, asset.mime FROM theme_revision_assets asset JOIN themes theme ON theme.id = asset.theme_id WHERE asset.theme_id = $1 AND asset.revision_digest = $2 AND theme.catalog_owner_key = $3 ORDER BY asset.path")
                    .bind(i64::from(theme_id)).bind(revision.as_ref()).bind(catalog_owner_key(owner)).fetch_all(&self.pool).await?;
                rows.into_iter().map(|(path, digest, mime)| digest.parse().map(|digest| ThemePackageAsset { path, digest, mime }).map_err(|_| sqlx::Error::RowNotFound)).collect()
            }
            async fn upsert_content_eligibility(&self, transaction: &mut WriteTransaction, eligibility: &ThemeContentEligibility) -> Result<(), sqlx::Error> { let connection = $conn(transaction)?; sqlx::query("INSERT INTO theme_content_eligibility (digest, mime, retained_until_unix_seconds) VALUES ($1, $2, $3) ON CONFLICT (digest) DO UPDATE SET mime = excluded.mime, retained_until_unix_seconds = CASE WHEN excluded.retained_until_unix_seconds > theme_content_eligibility.retained_until_unix_seconds THEN excluded.retained_until_unix_seconds ELSE theme_content_eligibility.retained_until_unix_seconds END").bind(eligibility.digest.as_ref()).bind(&eligibility.mime).bind(eligibility.retained_until_unix_seconds).execute(&mut *connection).await?; Ok(()) }
            async fn content_eligibility(&self, digest: &ThemeContentDigest) -> Result<Option<ThemeContentEligibility>, sqlx::Error> { let row: Option<(String, i64)> = sqlx::query_as("SELECT mime, retained_until_unix_seconds FROM theme_content_eligibility WHERE digest = $1").bind(digest.as_ref()).fetch_optional(&self.pool).await?; Ok(row.map(|(mime, retained_until_unix_seconds)| ThemeContentEligibility { digest: digest.clone(), mime, retained_until_unix_seconds })) }
            async fn list_content_eligibility(&self) -> Result<Vec<ThemeContentEligibility>, sqlx::Error> {
                let rows: Vec<(String, String, i64)> = sqlx::query_as("SELECT digest, mime, retained_until_unix_seconds FROM theme_content_eligibility ORDER BY digest").fetch_all(&self.pool).await?;
                rows.into_iter().map(|(digest, mime, retained_until_unix_seconds)| digest.parse().map(|digest| ThemeContentEligibility { digest, mime, retained_until_unix_seconds }).map_err(|_| sqlx::Error::RowNotFound)).collect()
            }
            async fn set_selection(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, selection: Option<PublicThemeSelection>) -> Result<(), sqlx::Error> {
                let catalog_owner_key = catalog_owner_key(owner);
                let connection = $conn(transaction)?;
                if let Some(PublicThemeSelection::Custom(theme_id)) = selection {
                    let selectable: (bool,) = sqlx::query_as(
                        "SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND catalog_owner_key = $2 AND current_revision_digest IS NOT NULL)",
                    )
                    .bind(i64::from(theme_id)).bind(&catalog_owner_key)
                    .fetch_one(&mut *connection).await?;
                    if !selectable.0 { return Err(sqlx::Error::RowNotFound); }
                }
                sqlx::query("DELETE FROM theme_selections WHERE catalog_owner_key = $1")
                    .bind(&catalog_owner_key).execute(&mut *connection).await?;
                if let Some(selection) = selection {
                    let (builtin_theme, theme_id) = match selection {
                        PublicThemeSelection::BuiltIn(theme) => (Some(theme.token()), None),
                        PublicThemeSelection::Custom(id) => (None, Some(i64::from(id))),
                    };
                    sqlx::query("INSERT INTO theme_selections (catalog_owner_key, builtin_theme, theme_id) VALUES ($1, $2, $3)")
                        .bind(&catalog_owner_key).bind(builtin_theme).bind(theme_id).execute(&mut *connection).await?;
                }
                Ok(())
            }
            async fn selection(&self, owner: ThemeOwner) -> Result<Option<PublicThemeSelection>, sqlx::Error> {
                let row: Option<(Option<String>, Option<i64>)> = sqlx::query_as(
                    "SELECT builtin_theme, theme_id FROM theme_selections WHERE catalog_owner_key = $1",
                )
                .bind(catalog_owner_key(owner))
                .fetch_optional(&self.pool)
                .await?;
                Ok(row.and_then(|(builtin, custom)| {
                    custom.map(|id| PublicThemeSelection::Custom(ThemeId::from(id))).or_else(|| {
                        builtin.and_then(|token| token.parse().ok().map(PublicThemeSelection::BuiltIn))
                    })
                }))
            }
            async fn remove_theme(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, theme_id: ThemeId, retained_until_unix_seconds: i64) -> Result<(), sqlx::Error> {
                let connection = $conn(transaction)?;
                let owner_key = catalog_owner_key(owner);
                // Keep the same site → owner → digest order used for revision
                // admission/detachment. The grouped UNION preserves multiplicity:
                // one digest may occur in several revisions or assets.
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&owner_key).execute(&mut *connection).await?;
                let owned: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND catalog_owner_key = $2)").bind(i64::from(theme_id)).bind(&owner_key).fetch_one(&mut *connection).await?;
                if !owned.0 { return Err(sqlx::Error::RowNotFound); }
                let draft_charge: (String, i64) = sqlx::query_as("SELECT source_digest, logical_bytes FROM theme_draft_charges WHERE theme_id = $1").bind(i64::from(theme_id)).fetch_one(&mut *connection).await?;
                let draft_refs: (i64,) = sqlx::query_as("SELECT live_references FROM theme_draft_content_charges WHERE source_digest = $1").bind(&draft_charge.0).fetch_one(&mut *connection).await?;
                let owner_bytes: (i64,) = sqlx::query_as("SELECT logical_bytes FROM theme_owner_quotas WHERE catalog_owner_key = $1").bind(&owner_key).fetch_one(&mut *connection).await?;
                if owner_bytes.0 < draft_charge.1 { return Err(sqlx::Error::RowNotFound); }
                sqlx::query("UPDATE theme_owner_quotas SET logical_bytes = logical_bytes - $1 WHERE catalog_owner_key = $2").bind(draft_charge.1).bind(&owner_key).execute(&mut *connection).await?;
                if draft_refs.0 == 1 {
                    let site_bytes: (i64,) = sqlx::query_as("SELECT physical_bytes FROM theme_site_quota WHERE singleton = 1").fetch_one(&mut *connection).await?;
                    if site_bytes.0 < draft_charge.1 { return Err(sqlx::Error::RowNotFound); }
                    sqlx::query("UPDATE theme_site_quota SET physical_bytes = physical_bytes - $1 WHERE singleton = 1").bind(draft_charge.1).execute(&mut *connection).await?;
                    sqlx::query("DELETE FROM theme_draft_content_charges WHERE source_digest = $1 AND live_references = 1").bind(&draft_charge.0).execute(&mut *connection).await?;
                } else {
                    sqlx::query("UPDATE theme_draft_content_charges SET live_references = live_references - 1 WHERE source_digest = $1 AND live_references > 1").bind(&draft_charge.0).execute(&mut *connection).await?;
                }
                let (revision_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM theme_revisions WHERE theme_id = $1").bind(i64::from(theme_id)).fetch_one(&mut *connection).await?;
                let charges: Vec<(String, i64)> = sqlx::query_as("SELECT content_digest, COUNT(*) FROM (SELECT revision.digest AS revision_digest, revision.stylesheet_digest AS content_digest FROM theme_revisions revision WHERE revision.theme_id = $1 UNION ALL SELECT asset.revision_digest, asset.digest AS content_digest FROM theme_revision_assets asset WHERE asset.theme_id = $1) content GROUP BY content_digest ORDER BY content_digest").bind(i64::from(theme_id)).fetch_all(&mut *connection).await?;
                for (digest, _) in &charges {
                    sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(digest).execute(&mut *connection).await?;
                    sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references WHERE catalog_owner_key = $1 AND digest = $2").bind(&owner_key).bind(digest).execute(&mut *connection).await?;
                }
                if revision_count > 0 {
                    let site = sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions - $1 WHERE singleton = 1 AND retained_revisions >= $1").bind(revision_count).execute(&mut *connection).await?;
                    let owner_quota = sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions - $1 WHERE catalog_owner_key = $2 AND retained_revisions >= $1").bind(revision_count).bind(&owner_key).execute(&mut *connection).await?;
                    if site.rows_affected() != 1 || owner_quota.rows_affected() != 1 { return Err(sqlx::Error::RowNotFound); }
                }
                for (digest, count) in &charges {
                    let charged = sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references - $1 WHERE catalog_owner_key = $2 AND digest = $3 AND live_references >= $1").bind(count).bind(&owner_key).bind(digest).execute(&mut *connection).await?;
                    let eligible = sqlx::query("UPDATE theme_content_eligibility SET live_references = live_references - $1, retained_until_unix_seconds = CASE WHEN retained_until_unix_seconds > $2 THEN retained_until_unix_seconds ELSE $2 END WHERE digest = $3 AND live_references >= $1").bind(count).bind(retained_until_unix_seconds).bind(digest).execute(&mut *connection).await?;
                    if charged.rows_affected() != 1 || eligible.rows_affected() != 1 { return Err(sqlx::Error::RowNotFound); }
                }
                let detached_selection = sqlx::query("DELETE FROM theme_selections WHERE theme_id = $1").bind(i64::from(theme_id)).execute(&mut *connection).await?;
                if owner == ThemeOwner::Site && detached_selection.rows_affected() == 1 {
                    sqlx::query("INSERT INTO theme_selections (catalog_owner_key, builtin_theme, theme_id) VALUES ('site', 'studio', NULL) ON CONFLICT (catalog_owner_key) DO UPDATE SET builtin_theme = excluded.builtin_theme, theme_id = NULL").execute(&mut *connection).await?;
                }
                let removed = sqlx::query("DELETE FROM themes WHERE id = $1 AND catalog_owner_key = $2").bind(i64::from(theme_id)).bind(&owner_key).execute(&mut *connection).await?;
                let active = sqlx::query("UPDATE theme_owner_quotas SET active_themes = active_themes - 1 WHERE catalog_owner_key = $1 AND active_themes > 0").bind(&owner_key).execute(&mut *connection).await?;
                if removed.rows_affected() != 1 || active.rows_affected() != 1 { return Err(sqlx::Error::RowNotFound); }
                Ok(())
            }
            async fn replace_role_binding(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, binding: &ThemeRoleBinding) -> Result<(), sqlx::Error> {
                if let ThemeRoleBinding::Media { user_id, .. } = binding
                    && author_user_id(owner).is_some_and(|author| i64::from(*user_id) != author)
                {
                    return Err(sqlx::Error::RowNotFound);
                }
                let connection = $conn(transaction)?;
                let owned: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND catalog_owner_key = $2)")
                    .bind(i64::from(binding.theme_id())).bind(catalog_owner_key(owner)).fetch_one(&mut *connection).await?;
                if !owned.0 { return Err(sqlx::Error::RowNotFound); }
                if let ThemeRoleBinding::PackageAsset { package_path, .. } = binding {
                    let valid: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_draft_assets WHERE theme_id = $1 AND path = $2 AND mime LIKE 'image/%')")
                        .bind(i64::from(binding.theme_id()))
                        .bind(package_path)
                        .fetch_one(&mut *connection)
                        .await?;
                    if !valid.0 {
                        return Err(sqlx::Error::RowNotFound);
                    }
                }
                if let ThemeRoleBinding::Media { user_id, media, .. } = binding {
                    let valid: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM media WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4)")
                        .bind(i64::from(*user_id))
                        .bind(media.source.as_ref())
                        .bind(media.sha256.as_ref())
                        .bind(media.filename.as_ref())
                        .fetch_one(&mut *connection)
                        .await?;
                    if !valid.0 {
                        return Err(sqlx::Error::RowNotFound);
                    }
                }
                if binding.role() == ThemeImageRole::Header
                    && !matches!(binding, ThemeRoleBinding::HeaderPool { .. })
                {
                    sqlx::query("DELETE FROM theme_header_pool WHERE theme_id = $1").bind(i64::from(binding.theme_id())).execute(&mut *connection).await?;
                }
                let columns = binding_columns(binding);
                sqlx::query("INSERT INTO theme_role_bindings (theme_id, role, mode, package_path, media_user_id, media_source, media_digest, media_filename, pool_revision_digest, shuffle_seed) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (theme_id, role) DO UPDATE SET mode = excluded.mode, package_path = excluded.package_path, media_user_id = excluded.media_user_id, media_source = excluded.media_source, media_digest = excluded.media_digest, media_filename = excluded.media_filename, pool_revision_digest = excluded.pool_revision_digest, shuffle_seed = excluded.shuffle_seed")
                    .bind(i64::from(binding.theme_id())).bind(binding.role().token()).bind(columns.mode).bind(columns.package_path).bind(columns.media_user_id).bind(columns.media_source).bind(columns.media_digest).bind(columns.media_filename).bind(columns.pool_revision).bind(columns.shuffle_seed).execute(&mut *connection).await?;
                Ok(())
            }
            async fn role_binding(&self, owner: ThemeOwner, theme_id: ThemeId, role: ThemeImageRole) -> Result<Option<ThemeRoleBinding>, sqlx::Error> {
                let row: Option<ThemeRoleBindingRow> = sqlx::query_as("SELECT b.mode, b.package_path, b.media_user_id, b.media_source, b.media_digest, b.media_filename, b.pool_revision_digest AS pool_revision, b.shuffle_seed FROM theme_role_bindings b JOIN themes t ON t.id = b.theme_id WHERE b.theme_id = $1 AND b.role = $2 AND t.catalog_owner_key = $3").bind(i64::from(theme_id)).bind(role.token()).bind(catalog_owner_key(owner)).fetch_optional(&self.pool).await?;
                row.map(|row| binding_from_row(theme_id, role, row)).transpose()
            }
            async fn replace_header_pool(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, theme_id: ThemeId, entries: &[ThemeHeaderPoolEntry]) -> Result<(), sqlx::Error> {
                let author_user_id = author_user_id(owner);
                if entries.is_empty() || author_user_id.is_some() && entries.iter().any(|entry| entry.media_user_id.map(i64::from) != author_user_id && entry.media_user_id.is_some()) {
                    return Err(sqlx::Error::RowNotFound);
                }
                let connection = $conn(transaction)?;
                let owned: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND catalog_owner_key = $2)").bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).fetch_one(&mut *connection).await?;
                if !owned.0 { return Err(sqlx::Error::RowNotFound); }
                let mut canonical = Vec::with_capacity(entries.len());
                let mut encodings = BTreeSet::new();
                for entry in entries {
                    let encoding = header_pool_entry_encoding(entry)?;
                    if !encodings.insert(encoding.clone()) {
                        return Err(sqlx::Error::RowNotFound);
                    }
                    let valid: (bool,) = match (&entry.package_path, entry.media_user_id, &entry.media_source, &entry.media_digest, &entry.media_filename) {
                        (Some(path), None, None, None, None) => sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_draft_assets WHERE theme_id = $1 AND path = $2 AND mime LIKE 'image/%')").bind(i64::from(theme_id)).bind(path).fetch_one(&mut *connection).await?,
                        (None, Some(user_id), Some(source), Some(digest), Some(filename)) => sqlx::query_as("SELECT EXISTS (SELECT 1 FROM media WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4)").bind(i64::from(user_id)).bind(source).bind(digest.as_ref()).bind(filename).fetch_one(&mut *connection).await?,
                        _ => return Err(sqlx::Error::RowNotFound),
                    };
                    if !valid.0 {
                        return Err(sqlx::Error::RowNotFound);
                    }
                    canonical.push((encoding, entry));
                }
                canonical.sort_by(|left, right| left.0.cmp(&right.0));
                sqlx::query("DELETE FROM theme_header_pool WHERE theme_id = $1").bind(i64::from(theme_id)).execute(&mut *connection).await?;
                for (ordinal, (_, entry)) in canonical.into_iter().enumerate() {
                    let ordinal = i64::try_from(ordinal).map_err(|_| sqlx::Error::RowNotFound)?;
                    sqlx::query("INSERT INTO theme_header_pool (theme_id, entry_ordinal, package_path, media_user_id, media_source, media_digest, media_filename) VALUES ($1, $2, $3, $4, $5, $6, $7)").bind(i64::from(theme_id)).bind(ordinal).bind(&entry.package_path).bind(entry.media_user_id.map(i64::from)).bind(&entry.media_source).bind(entry.media_digest.as_ref().map(AsRef::as_ref)).bind(&entry.media_filename).execute(&mut *connection).await?;
                }
                Ok(())
            }
            async fn header_pool(&self, owner: ThemeOwner, theme_id: ThemeId) -> Result<Vec<ThemeHeaderPoolEntry>, sqlx::Error> {
                let rows: Vec<(i64, Option<String>, Option<i64>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as("SELECT p.entry_ordinal, p.package_path, p.media_user_id, p.media_source, p.media_digest, p.media_filename FROM theme_header_pool p JOIN themes t ON t.id = p.theme_id WHERE p.theme_id = $1 AND t.catalog_owner_key = $2 ORDER BY p.entry_ordinal").bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).fetch_all(&self.pool).await?;
                Ok(rows.into_iter().map(|(ordinal, package_path, media_user_id, media_source, media_digest, media_filename)| ThemeHeaderPoolEntry { ordinal, package_path, media_user_id: media_user_id.map(UserId::from), media_source, media_digest: media_digest.and_then(|value| value.parse().ok()), media_filename }).collect())
            }
            async fn locked_media_references(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, theme_id: ThemeId) -> Result<Vec<ThemeMediaReference>, sqlx::Error> {
                let connection = $conn(transaction)?;
                let locked = sqlx::query("UPDATE themes SET id = id WHERE id = $1 AND catalog_owner_key = $2")
                    .bind(i64::from(theme_id))
                    .bind(catalog_owner_key(owner))
                    .execute(&mut *connection)
                    .await?;
                if locked.rows_affected() != 1 {
                    return Err(sqlx::Error::RowNotFound);
                }
                let rows: Vec<(i64, String, String, String)> = sqlx::query_as("SELECT media_user_id, media_source, media_digest, media_filename FROM (SELECT media_user_id, media_source, media_digest, media_filename FROM theme_role_bindings WHERE theme_id = $1 AND media_user_id IS NOT NULL UNION SELECT media_user_id, media_source, media_digest, media_filename FROM theme_header_pool WHERE theme_id = $1 AND media_user_id IS NOT NULL) theme_media ORDER BY media_digest, media_source, media_filename, media_user_id")
                    .bind(i64::from(theme_id))
                    .fetch_all(&mut *connection)
                    .await?;
                rows.into_iter()
                    .map(|(user_id, source, digest, filename)| {
                        Ok(ThemeMediaReference {
                            user_id: UserId::from(user_id),
                            media: MediaRef {
                                source: source.parse::<MediaSource>().map_err(|_| sqlx::Error::RowNotFound)?,
                                sha256: digest.parse::<ContentHash>().map_err(|_| sqlx::Error::RowNotFound)?,
                                filename: filename.parse::<Filename>().map_err(|_| sqlx::Error::RowNotFound)?,
                            },
                        })
                    })
                    .collect()
            }
            async fn shuffle_header_pool(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, theme_id: ThemeId, shuffle_seed: [u8; 32]) -> Result<(), sqlx::Error> {
                let connection = $conn(transaction)?;
                let updated = sqlx::query("UPDATE theme_role_bindings SET shuffle_seed = $1 WHERE theme_id = $2 AND role = 'header' AND mode = 'pool' AND EXISTS (SELECT 1 FROM themes WHERE id = $2 AND catalog_owner_key = $3)")
                    .bind(Vec::from(shuffle_seed)).bind(i64::from(theme_id)).bind(catalog_owner_key(owner)).execute(&mut *connection).await?;
                if updated.rows_affected() != 1 { return Err(sqlx::Error::RowNotFound); }
                Ok(())
            }
            async fn owner_quota(&self, owner: ThemeOwner) -> Result<Option<ThemeOwnerQuota>, sqlx::Error> {
                let row: Option<(i64, i64, i64)> = sqlx::query_as("SELECT active_themes, retained_revisions, logical_bytes FROM theme_owner_quotas WHERE catalog_owner_key = $1").bind(catalog_owner_key(owner)).fetch_optional(&self.pool).await?;
                Ok(row.map(|(active_themes, retained_revisions, logical_bytes)| ThemeOwnerQuota { active_themes, retained_revisions, logical_bytes }))
            }
            async fn site_quota(&self) -> Result<ThemeSiteQuota, sqlx::Error> {
                let (retained_revisions, physical_bytes): (i64, i64) = sqlx::query_as("SELECT retained_revisions, physical_bytes FROM theme_site_quota WHERE singleton = 1").fetch_one(&self.pool).await?;
                Ok(ThemeSiteQuota { retained_revisions, physical_bytes })
            }
            async fn admit_theme(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, limits: ThemeQuotaLimits) -> Result<(), sqlx::Error> {
                let connection = $conn(transaction)?;
                let key = catalog_owner_key(owner);
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                let result = sqlx::query("INSERT INTO theme_owner_quotas (catalog_owner_key, active_themes) VALUES ($1, 1) ON CONFLICT (catalog_owner_key) DO UPDATE SET active_themes = theme_owner_quotas.active_themes + 1 WHERE theme_owner_quotas.active_themes < $2").bind(&key).bind(limits.active_themes).execute(&mut *connection).await?;
                if result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                Ok(())
            }
            async fn admit_publication<'a>(&self, transaction: &mut WriteTransaction, admission: ThemePublicationAdmission<'a>) -> Result<(), sqlx::Error> {
                let ThemePublicationAdmission { owner, limits, revision, assets, eligibilities, charges } = admission;
                if charges.windows(2).any(|pair| pair[0].digest.as_ref() >= pair[1].digest.as_ref()) {
                    return Err(sqlx::Error::RowNotFound);
                }
                let connection = $conn(transaction)?;
                let key = catalog_owner_key(owner);
                // This is the shared site → owner → sorted-content lock order for
                // both backends. PostgreSQL's no-op updates take row locks;
                // SQLite's write transaction serializes the same critical section.
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&key).execute(&mut *connection).await?;
                for charge in charges {
                    sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                }
                let current: Option<(Option<String>,)> = sqlx::query_as("SELECT current_revision_digest FROM themes WHERE id = $1 AND catalog_owner_key = $2").bind(i64::from(revision.theme_id)).bind(&key).fetch_optional(&mut *connection).await?;
                match current {
                    Some((Some(digest),)) if digest == revision.digest.as_ref() => return Ok(()),
                    Some(_) => {}
                    None => return Err(sqlx::Error::RowNotFound),
                }
                // Bindings name assets in the candidate revision, not the revision
                // that happened to be current when the draft was edited.  This check
                // deliberately executes in the publication transaction before the
                // current-revision pointer is advanced.
                let candidate_paths = assets.iter().map(|asset| asset.path.as_str()).collect::<BTreeSet<_>>();
                let role_paths: Vec<(String,)> = sqlx::query_as("SELECT package_path FROM theme_role_bindings WHERE theme_id = $1 AND mode = 'package_asset' AND package_path IS NOT NULL")
                    .bind(i64::from(revision.theme_id)).fetch_all(&mut *connection).await?;
                let pool_paths: Vec<(String,)> = sqlx::query_as("SELECT package_path FROM theme_header_pool WHERE theme_id = $1 AND package_path IS NOT NULL")
                    .bind(i64::from(revision.theme_id)).fetch_all(&mut *connection).await?;
                if role_paths.iter().chain(pool_paths.iter()).any(|(path,)| !candidate_paths.contains(path.as_str())) {
                    return Err(sqlx::Error::RowNotFound);
                }
                for eligibility in eligibilities {
                    sqlx::query("INSERT INTO theme_content_eligibility (digest, mime, retained_until_unix_seconds) VALUES ($1, $2, $3) ON CONFLICT (digest) DO UPDATE SET mime = excluded.mime, retained_until_unix_seconds = CASE WHEN excluded.retained_until_unix_seconds > theme_content_eligibility.retained_until_unix_seconds THEN excluded.retained_until_unix_seconds ELSE theme_content_eligibility.retained_until_unix_seconds END").bind(eligibility.digest.as_ref()).bind(&eligibility.mime).bind(eligibility.retained_until_unix_seconds).execute(&mut *connection).await?;
                }
                self.attach_revision_content(transaction, owner, limits, charges).await?;
                self.record_revision(transaction, owner, revision, assets).await
            }
            async fn attach_revision_content(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, limits: ThemeQuotaLimits, charges: &[ThemeContentCharge]) -> Result<(), sqlx::Error> {
                if charges.windows(2).any(|pair| pair[0].digest.as_ref() >= pair[1].digest.as_ref()) { return Err(sqlx::Error::RowNotFound); }
                let connection = $conn(transaction)?;
                let key = catalog_owner_key(owner);
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&key).execute(&mut *connection).await?;
                for charge in charges {
                    sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                }
                for charge in charges {
                    sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references WHERE catalog_owner_key = $1 AND digest = $2").bind(&key).bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                }
                let site_result = sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions + 1 WHERE singleton = 1 AND retained_revisions < $1").bind(limits.site_retained_revisions).execute(&mut *connection).await?;
                if site_result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                let owner_result = sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions + 1 WHERE catalog_owner_key = $1 AND retained_revisions < $2").bind(&key).bind(limits.retained_revisions).execute(&mut *connection).await?;
                if owner_result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                for charge in charges {
                    let eligibility: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_content_eligibility WHERE digest = $1)").bind(charge.digest.as_ref()).fetch_one(&mut *connection).await?;
                    if !eligibility.0 { return Err(sqlx::Error::RowNotFound); }
                    let already_charged: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_retained_content_charges WHERE digest = $1)").bind(charge.digest.as_ref()).fetch_one(&mut *connection).await?;
                    let owner_charged: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_retained_content_charges WHERE catalog_owner_key = $1 AND digest = $2)").bind(&key).bind(charge.digest.as_ref()).fetch_one(&mut *connection).await?;
                    sqlx::query("INSERT INTO theme_retained_content_charges (catalog_owner_key, digest, logical_bytes, physical_bytes, live_references) VALUES ($1, $2, $3, $4, 1) ON CONFLICT (catalog_owner_key, digest) DO UPDATE SET live_references = theme_retained_content_charges.live_references + 1").bind(&key).bind(charge.digest.as_ref()).bind(charge.logical_bytes).bind(charge.physical_bytes).execute(&mut *connection).await?;
                    if !owner_charged.0 {
                        let result = sqlx::query("UPDATE theme_owner_quotas SET logical_bytes = logical_bytes + $1 WHERE catalog_owner_key = $2 AND logical_bytes <= $3 - $1").bind(charge.logical_bytes).bind(&key).bind(limits.logical_bytes).execute(&mut *connection).await?;
                        if result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                        if !already_charged.0 {
                            let result = sqlx::query("UPDATE theme_site_quota SET physical_bytes = physical_bytes + $1 WHERE singleton = 1 AND physical_bytes <= $2 - $1").bind(charge.physical_bytes).bind(limits.site_physical_bytes).execute(&mut *connection).await?;
                            if result.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                        }
                    }
                    let reference = sqlx::query("UPDATE theme_content_eligibility SET live_references = live_references + 1 WHERE digest = $1").bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                    if reference.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                }
                Ok(())
            }
            async fn detach_revision_content(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, charges: &[ThemeContentCharge], retained_until_unix_seconds: i64) -> Result<(), sqlx::Error> {
                if charges.windows(2).any(|pair| pair[0].digest.as_ref() >= pair[1].digest.as_ref()) { return Err(sqlx::Error::RowNotFound); }
                let connection = $conn(transaction)?;
                let key = catalog_owner_key(owner);
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&key).execute(&mut *connection).await?;
                for charge in charges {
                    sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                }
                for charge in charges {
                    sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references WHERE catalog_owner_key = $1 AND digest = $2").bind(&key).bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                }
                let site = sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions - 1 WHERE singleton = 1 AND retained_revisions > 0").execute(&mut *connection).await?;
                let owner = sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions - 1 WHERE catalog_owner_key = $1 AND retained_revisions > 0").bind(&key).execute(&mut *connection).await?;
                if site.rows_affected() == 0 || owner.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                for charge in charges {
                    let charged = sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references - 1 WHERE catalog_owner_key = $1 AND digest = $2 AND live_references > 0").bind(&key).bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                    if charged.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                    let reference = sqlx::query("UPDATE theme_content_eligibility SET live_references = live_references - 1, retained_until_unix_seconds = CASE WHEN retained_until_unix_seconds > $1 THEN retained_until_unix_seconds ELSE $1 END WHERE digest = $2 AND live_references > 0").bind(retained_until_unix_seconds).bind(charge.digest.as_ref()).execute(&mut *connection).await?;
                    if reference.rows_affected() == 0 { return Err(sqlx::Error::RowNotFound); }
                }
                Ok(())
            }
            async fn collect_retained_content(&self, transaction: &mut WriteTransaction, owner: ThemeOwner, digest: &ThemeContentDigest, now_unix_seconds: i64) -> Result<(), sqlx::Error> {
                let connection = $conn(transaction)?;
                let key = catalog_owner_key(owner);
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET retained_revisions = retained_revisions WHERE catalog_owner_key = $1").bind(&key).execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(digest.as_ref()).execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_retained_content_charges SET live_references = live_references WHERE catalog_owner_key = $1 AND digest = $2").bind(&key).bind(digest.as_ref()).execute(&mut *connection).await?;
                let charge: Option<(i64, i64)> = sqlx::query_as("SELECT c.logical_bytes, c.physical_bytes FROM theme_retained_content_charges c JOIN theme_content_eligibility e ON e.digest = c.digest WHERE c.catalog_owner_key = $1 AND c.digest = $2 AND c.live_references = 0 AND e.live_references = 0 AND e.retained_until_unix_seconds <= $3").bind(&key).bind(digest.as_ref()).bind(now_unix_seconds).fetch_optional(&mut *connection).await?;
                let Some((logical_bytes, physical_bytes)) = charge else { return Err(sqlx::Error::RowNotFound); };
                sqlx::query("DELETE FROM theme_retained_content_charges WHERE catalog_owner_key = $1 AND digest = $2 AND live_references = 0").bind(&key).bind(digest.as_ref()).execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_owner_quotas SET logical_bytes = logical_bytes - $1 WHERE catalog_owner_key = $2 AND logical_bytes >= $1").bind(logical_bytes).bind(&key).execute(&mut *connection).await?;
                let remaining: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM theme_retained_content_charges WHERE digest = $1)").bind(digest.as_ref()).fetch_one(&mut *connection).await?;
                if !remaining.0 {
                    sqlx::query("DELETE FROM theme_content_eligibility WHERE digest = $1 AND live_references = 0 AND retained_until_unix_seconds <= $2").bind(digest.as_ref()).bind(now_unix_seconds).execute(&mut *connection).await?;
                    sqlx::query("UPDATE theme_site_quota SET physical_bytes = physical_bytes - $1 WHERE singleton = 1 AND physical_bytes >= $1").bind(physical_bytes).execute(&mut *connection).await?;
                }
                Ok(())
            }
            async fn expired_retained_content(&self, now_unix_seconds: i64) -> Result<Vec<(ThemeOwner, ThemeContentDigest)>, sqlx::Error> {
                let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.catalog_owner_key, c.digest FROM theme_retained_content_charges c JOIN theme_content_eligibility e ON e.digest = c.digest WHERE c.live_references = 0 AND e.live_references = 0 AND e.retained_until_unix_seconds <= $1 ORDER BY c.catalog_owner_key, c.digest").bind(now_unix_seconds).fetch_all(&self.pool).await?;
                rows.into_iter().map(|(owner, digest)| Ok((theme_owner_from_key(&owner).ok_or(sqlx::Error::RowNotFound)?, digest.parse().map_err(|_| sqlx::Error::RowNotFound)?))).collect() // cov:ignore
            }
        }
    }
}

fn header_pool_entry_encoding(entry: &ThemeHeaderPoolEntry) -> Result<Vec<u8>, sqlx::Error> {
    let mut encoded = Vec::new();
    match (
        &entry.package_path,
        &entry.media_source,
        &entry.media_digest,
        &entry.media_filename,
    ) {
        (Some(path), None, None, None) if entry.media_user_id.is_none() => {
            encoded.push(0);
            push_theme_pool_field(&mut encoded, path.as_bytes());
        }
        (None, Some(source), Some(digest), Some(filename)) if entry.media_user_id.is_some() => {
            encoded.push(1);
            push_theme_pool_field(&mut encoded, source.as_bytes());
            let raw = theme_digest_bytes(digest.as_ref())?;
            encoded.extend(raw);
            push_theme_pool_field(&mut encoded, filename.as_bytes());
        }
        _ => return Err(sqlx::Error::RowNotFound),
    }
    Ok(encoded)
}

#[derive(Debug, macros::SqlxBridge)]
struct StoredThemeBindingMode(String);

#[derive(Debug, macros::SqlxBridge)]
struct StoredThemePackagePath(String);

#[derive(Debug, macros::SqlxBridge)]
struct StoredThemePoolRevision(String);

#[derive(Debug, macros::SqlxBridge)]
struct StoredThemeShuffleSeed(Vec<u8>);

#[derive(sqlx::FromRow)]
struct ThemeRoleBindingRow {
    mode: StoredThemeBindingMode,
    package_path: Option<StoredThemePackagePath>,
    media_user_id: Option<UserId>,
    media_source: Option<MediaSource>,
    media_digest: Option<ContentHash>,
    media_filename: Option<Filename>,
    pool_revision: Option<StoredThemePoolRevision>,
    shuffle_seed: Option<StoredThemeShuffleSeed>,
}

fn no_binding_columns_present(columns: [bool; 7]) -> bool {
    !columns.into_iter().any(std::convert::identity)
}

fn binding_from_row(
    theme_id: ThemeId,
    role: ThemeImageRole,
    row: ThemeRoleBindingRow,
) -> Result<ThemeRoleBinding, sqlx::Error> {
    let ThemeRoleBindingRow {
        mode,
        package_path,
        media_user_id,
        media_source,
        media_digest,
        media_filename,
        pool_revision,
        shuffle_seed,
    } = row;
    let no_media_or_pool = no_binding_columns_present([
        media_user_id.is_some(),
        media_source.is_some(),
        media_digest.is_some(),
        media_filename.is_some(),
        pool_revision.is_some(),
        shuffle_seed.is_some(),
        false,
    ]);
    let no_binding_columns = no_binding_columns_present([
        package_path.is_some(),
        media_user_id.is_some(),
        media_source.is_some(),
        media_digest.is_some(),
        media_filename.is_some(),
        pool_revision.is_some(),
        shuffle_seed.is_some(),
    ]);
    match mode.0.as_str() {
        "packaged_default" if no_binding_columns => {
            Ok(ThemeRoleBinding::PackagedDefault { theme_id, role })
        }
        "explicit_absent" if no_binding_columns => {
            Ok(ThemeRoleBinding::ExplicitAbsent { theme_id, role })
        }
        "package_asset" if no_media_or_pool => package_path
            .map(|package_path| ThemeRoleBinding::PackageAsset {
                theme_id,
                role,
                package_path: package_path.0,
            })
            .ok_or(sqlx::Error::RowNotFound),
        "media"
            if no_binding_columns_present([
                package_path.is_some(),
                pool_revision.is_some(),
                shuffle_seed.is_some(),
                false,
                false,
                false,
                false,
            ]) =>
        {
            let (Some(user_id), Some(source), Some(digest), Some(filename)) =
                (media_user_id, media_source, media_digest, media_filename)
            else {
                return Err(sqlx::Error::RowNotFound);
            };
            Ok(ThemeRoleBinding::Media {
                theme_id,
                role,
                user_id,
                media: MediaRef {
                    source,
                    sha256: digest,
                    filename,
                },
            })
        }
        "pool"
            if role == ThemeImageRole::Header
                && no_binding_columns_present([
                    package_path.is_some(),
                    media_user_id.is_some(),
                    media_source.is_some(),
                    media_digest.is_some(),
                    media_filename.is_some(),
                    false,
                    false,
                ]) =>
        {
            let (Some(pool_revision), Some(shuffle_seed)) = (pool_revision, shuffle_seed) else {
                return Err(sqlx::Error::RowNotFound);
            };
            Ok(ThemeRoleBinding::HeaderPool {
                theme_id,
                pool_revision: pool_revision
                    .0
                    .parse()
                    .map_err(|_| sqlx::Error::RowNotFound)?,
                shuffle_seed: shuffle_seed
                    .0
                    .try_into()
                    .map_err(|_| sqlx::Error::RowNotFound)?,
            })
        }
        _ => Err(sqlx::Error::RowNotFound),
    }
}

struct ThemeRoleBindingColumns<'a> {
    mode: &'static str,
    package_path: Option<&'a str>,
    media_user_id: Option<i64>,
    media_source: Option<&'a str>,
    media_digest: Option<&'a str>,
    media_filename: Option<&'a str>,
    pool_revision: Option<&'a str>,
    shuffle_seed: Option<Vec<u8>>,
}

fn binding_columns(binding: &ThemeRoleBinding) -> ThemeRoleBindingColumns<'_> {
    match binding {
        ThemeRoleBinding::PackagedDefault { .. } => ThemeRoleBindingColumns {
            mode: "packaged_default",
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        },
        ThemeRoleBinding::ExplicitAbsent { .. } => ThemeRoleBindingColumns {
            mode: "explicit_absent",
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        },
        ThemeRoleBinding::PackageAsset { package_path, .. } => ThemeRoleBindingColumns {
            mode: "package_asset",
            package_path: Some(package_path),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        },
        ThemeRoleBinding::Media { user_id, media, .. } => ThemeRoleBindingColumns {
            mode: "media",
            package_path: None,
            media_user_id: Some(i64::from(*user_id)),
            media_source: Some(media.source.as_ref()),
            media_digest: Some(media.sha256.as_ref()),
            media_filename: Some(media.filename.as_ref()),
            pool_revision: None,
            shuffle_seed: None,
        },
        ThemeRoleBinding::HeaderPool {
            pool_revision,
            shuffle_seed,
            ..
        } => ThemeRoleBindingColumns {
            mode: "pool",
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: Some(pool_revision.as_ref()),
            shuffle_seed: Some(shuffle_seed.to_vec()),
        },
    }
}

fn push_theme_pool_field(target: &mut Vec<u8>, value: &[u8]) {
    target.extend((value.len() as u64).to_be_bytes());
    target.extend(value);
}

fn theme_digest_bytes(value: &str) -> Result<[u8; 32], sqlx::Error> {
    if value.len() != 64 {
        return Err(sqlx::Error::RowNotFound);
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| sqlx::Error::RowNotFound)?;
    }
    Ok(bytes)
}

fn author_user_id(owner: ThemeOwner) -> Option<i64> {
    match owner {
        ThemeOwner::Site => None,
        ThemeOwner::Author(user_id) => Some(i64::from(user_id)),
    }
}

fn draft_storage_bytes(draft: &ThemeDraft) -> Result<i64, sqlx::Error> {
    draft
        .assets
        .iter()
        .try_fold(
            draft
                .manifest
                .len()
                .checked_add(draft.stylesheet.len())
                .ok_or(sqlx::Error::RowNotFound)?,
            |total, asset| {
                total
                    .checked_add(asset.bytes.len())
                    .ok_or(sqlx::Error::RowNotFound)
            },
        )
        .and_then(|total| i64::try_from(total).map_err(|_| sqlx::Error::RowNotFound))
}

fn catalog_owner_key(owner: ThemeOwner) -> String {
    match owner {
        ThemeOwner::Site => "site".to_owned(),
        ThemeOwner::Author(user_id) => format!("user:{}", i64::from(user_id)),
    }
}

fn theme_owner_from_key(key: &str) -> Option<ThemeOwner> {
    match key {
        "site" => Some(ThemeOwner::Site),
        value => value
            .strip_prefix("user:")
            .and_then(|id| id.parse::<i64>().ok())
            .map(UserId::from)
            .map(ThemeOwner::Author),
    }
}

fn canonical_theme_name_key(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::{
        ids::{ThemeId, UserId},
        theme::{PublicThemeSelection, Theme, ThemeImageRole},
    };
    use rstest::*;
    use rstest_reuse::*;

    use super::*;
    use crate::test_support::{Backend, backends, confirmed};

    fn draft_limits() -> ThemeQuotaLimits {
        ThemeQuotaLimits {
            active_themes: 2,
            retained_revisions: 2,
            logical_bytes: 1_024,
            site_retained_revisions: 2,
            site_physical_bytes: 1_024,
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn theme_catalog_draft_selection_roles_pool_and_quotas_round_trip(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let themes = Arc::clone(&env.themes());
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: b"body{}".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: vec![
                ThemeDraftAsset {
                    path: "assets/logo.svg".to_owned(),
                    mime: "image/svg+xml".to_owned(),
                    bytes: b"<svg/>".to_vec(),
                    digest: "b".repeat(64).parse().unwrap(),
                },
                ThemeDraftAsset {
                    path: "assets/header.webp".to_owned(),
                    mime: "image/webp".to_owned(),
                    bytes: vec![1, 2, 3],
                    digest: "c".repeat(64).parse().unwrap(),
                },
            ],
        };
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "Paper",
                                &draft,
                                ThemeQuotaLimits {
                                    active_themes: 1,
                                    retained_revisions: 2,
                                    logical_bytes: 64,
                                    site_retained_revisions: 2,
                                    site_physical_bytes: 64,
                                },
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap()
                .unwrap()
                .assets,
            vec![
                ThemeDraftAsset {
                    path: "assets/header.webp".to_owned(),
                    mime: "image/webp".to_owned(),
                    bytes: vec![1, 2, 3],
                    digest: "c".repeat(64).parse().unwrap(),
                },
                ThemeDraftAsset {
                    path: "assets/logo.svg".to_owned(),
                    mime: "image/svg+xml".to_owned(),
                    bytes: b"<svg/>".to_vec(),
                    digest: "b".repeat(64).parse().unwrap(),
                },
            ]
        );
        let replacement = ThemeDraft {
            theme_id: created,
            manifest: b"{\"replacement\":true}".to_vec(),
            stylesheet: b"main{}".to_vec(),
            source_digest: "d".repeat(64).parse().unwrap(),
            assets: vec![ThemeDraftAsset {
                path: "assets/replacement.png".to_owned(),
                mime: "image/png".to_owned(),
                bytes: vec![4, 5, 6],
                digest: "e".repeat(64).parse().unwrap(),
            }],
        };
        let expected_replacement = replacement.clone();
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_draft(
                                transaction,
                                ThemeOwner::Site,
                                &replacement,
                                ThemeQuotaLimits {
                                    active_themes: 1,
                                    retained_revisions: 2,
                                    logical_bytes: 64,
                                    site_retained_revisions: 2,
                                    site_physical_bytes: 64,
                                },
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap(),
            Some(expected_replacement)
        );

        let themes = Arc::clone(&env.themes());
        let revision = ThemeRevision {
            theme_id: created,
            digest: "b".repeat(64).parse().unwrap(),
            stylesheet_digest: "c".repeat(64).parse().unwrap(),
            manifest: b"{}".to_vec(),
        };
        let assets = vec![
            ThemePackageAsset {
                path: "assets/header.webp".to_owned(),
                digest: "d".repeat(64).parse().unwrap(),
                mime: "image/webp".to_owned(),
            },
            ThemePackageAsset {
                path: "assets/logo.webp".to_owned(),
                digest: "e".repeat(64).parse().unwrap(),
                mime: "image/webp".to_owned(),
            },
        ];
        let expected_assets = assets.clone();
        let revision_digest = revision.digest.clone();
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .record_revision(transaction, ThemeOwner::Site, &revision, &assets)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .revision_assets(ThemeOwner::Site, created, &revision_digest)
                .await
                .unwrap(),
            expected_assets
        );

        let themes = Arc::clone(&env.themes());
        let binding = ThemeRoleBinding::PackageAsset {
            theme_id: created,
            role: ThemeImageRole::Logo,
            package_path: "assets/replacement.png".into(),
        };
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_role_binding(transaction, ThemeOwner::Site, &binding)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(matches!(
            env.themes()
                .role_binding(ThemeOwner::Site, created, ThemeImageRole::Logo)
                .await
                .unwrap(),
            Some(ThemeRoleBinding::PackageAsset { package_path, .. })
                if package_path == "assets/replacement.png"
        ));

        let themes = Arc::clone(&env.themes());
        let pool = vec![ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("assets/replacement.png".into()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_header_pool(transaction, ThemeOwner::Site, created, &pool)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .header_pool(ThemeOwner::Site, created)
                .await
                .unwrap()
                .len(),
            1
        );

        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Site,
                                Some(PublicThemeSelection::BuiltIn(Theme::Reader)),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes().selection(ThemeOwner::Site).await.unwrap(),
            Some(PublicThemeSelection::BuiltIn(Theme::Reader))
        );

        assert_eq!(
            env.themes().owner_quota(ThemeOwner::Site).await.unwrap(),
            Some(ThemeOwnerQuota {
                active_themes: 1,
                retained_revisions: 0,
                logical_bytes: 29,
            })
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn draft_bytes_admit_replace_and_release_atomically(#[case] backend: Backend) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 2,
            retained_revisions: 1,
            logical_bytes: 4,
            site_retained_revisions: 1,
            site_physical_bytes: 4,
        };
        let draft = |id, digest: char, size| ThemeDraft {
            theme_id: ThemeId::from(id),
            manifest: vec![b'x'; size],
            stylesheet: Vec::new(),
            source_digest: digest.to_string().repeat(64).parse().unwrap(),
            assets: Vec::new(),
        };

        let rejected = draft(0, 'a', 5);
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| Box::pin(async move {
                    themes
                        .create_theme(transaction, ThemeOwner::Site, "Rejected", &rejected, limits)
                        .await
                }))
                .await
                .is_err()
        );
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .unwrap()
                .is_empty()
        );

        let initial = draft(0, 'b', 4);
        let themes = Arc::clone(&env.themes());
        let id = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(transaction, ThemeOwner::Site, "Draft", &initial, limits)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .unwrap()
                .unwrap()
                .logical_bytes,
            4
        );
        assert_eq!(env.themes().site_quota().await.unwrap().physical_bytes, 4);

        let growth = draft(id.into(), 'c', 5);
        let expected = env
            .themes()
            .get_draft(ThemeOwner::Site, id)
            .await
            .unwrap()
            .unwrap();
        let themes = Arc::clone(&env.themes());
        let error = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .replace_draft(transaction, ThemeOwner::Site, &growth, limits)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::WriteScopeError::Operation(ReplaceDraftError::QuotaExceeded)
        ));
        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, id)
                .await
                .unwrap()
                .unwrap(),
            expected
        );
        let missing = draft(999, 'e', 2);
        let themes = Arc::clone(&env.themes());
        let error = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .replace_draft(transaction, ThemeOwner::Site, &missing, limits)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::WriteScopeError::Operation(ReplaceDraftError::OwnerNotFound)
        ));

        let shrink = draft(id.into(), 'd', 2);
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_draft(transaction, ThemeOwner::Site, &shrink, limits)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .unwrap()
                .unwrap()
                .logical_bytes,
            2
        );
        assert_eq!(env.themes().site_quota().await.unwrap().physical_bytes, 2);

        let exact = draft(0, 'e', 2);
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(transaction, ThemeOwner::Site, "Exact", &exact, limits)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(env.themes().site_quota().await.unwrap().physical_bytes, 4);

        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .remove_theme(transaction, ThemeOwner::Site, id, 0)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .unwrap()
                .unwrap()
                .logical_bytes,
            2
        );
        assert_eq!(env.themes().site_quota().await.unwrap().physical_bytes, 2);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn catalog_names_are_trimmed_and_reject_empty_and_fixed_roles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 2,
            retained_revisions: 2,
            logical_bytes: 1_024,
            site_retained_revisions: 2,
            site_physical_bytes: 1_024,
        };
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: Vec::new(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: Vec::new(),
        };

        for name in ["  ", "tErMiNaL", "STUDIO", "reader"] {
            let themes = Arc::clone(&env.themes());
            let draft = draft.clone();
            assert!(
                env.write_scope()
                    .run(move |transaction| Box::pin(async move {
                        themes
                            .create_theme(transaction, ThemeOwner::Site, name, &draft, limits)
                            .await
                    }))
                    .await
                    .is_err()
            );
        }
        let themes = Arc::clone(&env.themes());
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "  Parchment  ",
                                &draft,
                                limits,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            env.themes().list_themes(ThemeOwner::Site).await.unwrap()[0].name,
            "Parchment"
        );

        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .rename_theme(transaction, ThemeOwner::Site, created, "  Canvas  ")
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        for name in ["", "  TeRmInAl  ", "studio", "READER"] {
            let themes = Arc::clone(&env.themes());
            assert!(
                env.write_scope()
                    .run(move |transaction| Box::pin(async move {
                        themes
                            .rename_theme(transaction, ThemeOwner::Site, created, name)
                            .await
                    }))
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            env.themes().list_themes(ThemeOwner::Site).await.unwrap()[0].name,
            "Canvas"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn fixed_roles_and_header_pools_reject_package_fonts(#[case] backend: Backend) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 1,
            logical_bytes: 1_024,
            site_retained_revisions: 1,
            site_physical_bytes: 1_024,
        };
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: Vec::new(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: vec![ThemeDraftAsset {
                path: "assets/type.woff2".to_owned(),
                mime: "font/woff2".to_owned(),
                bytes: vec![1, 2, 3],
                digest: "b".repeat(64).parse().unwrap(),
            }],
        };
        let themes = Arc::clone(&env.themes());
        let theme_id = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "Font package",
                                &draft,
                                limits,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        let binding = ThemeRoleBinding::PackageAsset {
            theme_id,
            role: ThemeImageRole::Logo,
            package_path: "assets/type.woff2".to_owned(),
        };
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| Box::pin(async move {
                    themes
                        .replace_role_binding(transaction, ThemeOwner::Site, &binding)
                        .await
                }))
                .await
                .is_err()
        );

        let pool = vec![ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("assets/type.woff2".to_owned()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| Box::pin(async move {
                    themes
                        .replace_header_pool(transaction, ThemeOwner::Site, theme_id, &pool)
                        .await
                }))
                .await
                .is_err()
        );
        assert!(
            env.themes()
                .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Logo)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            env.themes()
                .header_pool(ThemeOwner::Site, theme_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn draft_assets_round_trip_in_canonical_path_order(#[case] backend: Backend) {
        let env = backend.setup().await;
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: br#"{"name":"ordered"}"#.to_vec(),
            stylesheet: b"body { color: black; }".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: vec![
                ThemeDraftAsset {
                    path: "images/zebra.svg".to_owned(),
                    mime: "image/svg+xml".to_owned(),
                    bytes: b"<svg>zebra</svg>".to_vec(),
                    digest: "b".repeat(64).parse().unwrap(),
                },
                ThemeDraftAsset {
                    path: "images/apple.webp".to_owned(),
                    mime: "image/webp".to_owned(),
                    bytes: vec![0, 1, 2, 3],
                    digest: "c".repeat(64).parse().unwrap(),
                },
            ],
        };
        let themes = Arc::clone(&env.themes());
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "Ordered assets",
                                &draft,
                                draft_limits(),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap(),
            Some(ThemeDraft {
                theme_id: created,
                manifest: br#"{"name":"ordered"}"#.to_vec(),
                stylesheet: b"body { color: black; }".to_vec(),
                source_digest: "a".repeat(64).parse().unwrap(),
                assets: vec![
                    ThemeDraftAsset {
                        path: "images/apple.webp".to_owned(),
                        mime: "image/webp".to_owned(),
                        bytes: vec![0, 1, 2, 3],
                        digest: "c".repeat(64).parse().unwrap(),
                    },
                    ThemeDraftAsset {
                        path: "images/zebra.svg".to_owned(),
                        mime: "image/svg+xml".to_owned(),
                        bytes: b"<svg>zebra</svg>".to_vec(),
                        digest: "b".repeat(64).parse().unwrap(),
                    },
                ],
            })
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn replacing_draft_assets_removes_stale_assets_and_preserves_new_data(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let initial = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: b"body{}".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: vec![
                ThemeDraftAsset {
                    path: "images/stale-logo.svg".to_owned(),
                    mime: "image/svg+xml".to_owned(),
                    bytes: b"stale-logo".to_vec(),
                    digest: "b".repeat(64).parse().unwrap(),
                },
                ThemeDraftAsset {
                    path: "images/stale-header.webp".to_owned(),
                    mime: "image/webp".to_owned(),
                    bytes: b"stale-header".to_vec(),
                    digest: "c".repeat(64).parse().unwrap(),
                },
            ],
        };
        let themes = Arc::clone(&env.themes());
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "Replacement assets",
                                &initial,
                                draft_limits(),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let replacement = ThemeDraft {
            theme_id: created,
            manifest: br#"{"name":"replaced"}"#.to_vec(),
            stylesheet: b"main { display: block; }".to_vec(),
            source_digest: "d".repeat(64).parse().unwrap(),
            assets: vec![
                ThemeDraftAsset {
                    path: "images/new-zebra.png".to_owned(),
                    mime: "image/png".to_owned(),
                    bytes: vec![137, 80, 78, 71],
                    digest: "e".repeat(64).parse().unwrap(),
                },
                ThemeDraftAsset {
                    path: "images/new-apple.avif".to_owned(),
                    mime: "image/avif".to_owned(),
                    bytes: vec![0, 0, 0, 32, 102, 116, 121, 112],
                    digest: "f".repeat(64).parse().unwrap(),
                },
            ],
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_draft(
                                transaction,
                                ThemeOwner::Site,
                                &replacement,
                                draft_limits(),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap(),
            Some(ThemeDraft {
                theme_id: created,
                manifest: br#"{"name":"replaced"}"#.to_vec(),
                stylesheet: b"main { display: block; }".to_vec(),
                source_digest: "d".repeat(64).parse().unwrap(),
                assets: vec![
                    ThemeDraftAsset {
                        path: "images/new-apple.avif".to_owned(),
                        mime: "image/avif".to_owned(),
                        bytes: vec![0, 0, 0, 32, 102, 116, 121, 112],
                        digest: "f".repeat(64).parse().unwrap(),
                    },
                    ThemeDraftAsset {
                        path: "images/new-zebra.png".to_owned(),
                        mime: "image/png".to_owned(),
                        bytes: vec![137, 80, 78, 71],
                        digest: "e".repeat(64).parse().unwrap(),
                    },
                ],
            })
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn draft_assets_are_masked_from_other_owners(#[case] backend: Backend) {
        let env = backend.setup().await;
        let owner = ThemeOwner::Author(UserId::from(41));
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: b"body{}".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: vec![ThemeDraftAsset {
                path: "images/private.svg".to_owned(),
                mime: "image/svg+xml".to_owned(),
                bytes: b"<svg>private</svg>".to_vec(),
                digest: "b".repeat(64).parse().unwrap(),
            }],
        };
        let themes = Arc::clone(&env.themes());
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                owner,
                                "Private assets",
                                &draft,
                                draft_limits(),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            env.themes()
                .get_draft(owner, created)
                .await
                .unwrap()
                .unwrap()
                .assets,
            vec![ThemeDraftAsset {
                path: "images/private.svg".to_owned(),
                mime: "image/svg+xml".to_owned(),
                bytes: b"<svg>private</svg>".to_vec(),
                digest: "b".repeat(64).parse().unwrap(),
            }]
        );

        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Author(UserId::from(42)), created)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn draft_without_assets_decodes_as_an_empty_asset_list(#[case] backend: Backend) {
        let env = backend.setup().await;
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: br#"{"name":"empty"}"#.to_vec(),
            stylesheet: b"body { margin: 0; }".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: Vec::new(),
        };
        let themes = Arc::clone(&env.themes());
        let created = confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Site,
                                "Empty assets",
                                &draft,
                                draft_limits(),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            env.themes()
                .get_draft(ThemeOwner::Site, created)
                .await
                .unwrap(),
            Some(ThemeDraft {
                theme_id: created,
                manifest: br#"{"name":"empty"}"#.to_vec(),
                stylesheet: b"body { margin: 0; }".to_vec(),
                source_digest: "a".repeat(64).parse().unwrap(),
                assets: Vec::new(),
            })
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn lifecycle_retains_and_collects_content_after_eligibility_expires(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 1,
            logical_bytes: 10,
            site_retained_revisions: 1,
            site_physical_bytes: 10,
        };
        let digest = "c".repeat(64).parse::<ThemeContentDigest>().unwrap();
        let charge = ThemeContentCharge {
            digest: digest.clone(),
            logical_bytes: 4,
            physical_bytes: 5,
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .admit_theme(transaction, ThemeOwner::Site, limits)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let themes = Arc::clone(&env.themes());
        let eligibility = ThemeContentEligibility {
            digest: digest.clone(),
            mime: "text/plain".into(),
            retained_until_unix_seconds: 0,
        };
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .upsert_content_eligibility(transaction, &eligibility)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(
                                transaction,
                                ThemeOwner::Site,
                                limits,
                                &[charge],
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let themes = Arc::clone(&env.themes());
        let charge = ThemeContentCharge {
            digest: digest.clone(),
            logical_bytes: 4,
            physical_bytes: 5,
        };
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .detach_revision_content(transaction, ThemeOwner::Site, &[charge], 10)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let themes = Arc::clone(&env.themes());
        let digest_to_collect = digest.clone();
        let before_expiry = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .collect_retained_content(
                            transaction,
                            ThemeOwner::Site,
                            &digest_to_collect,
                            9,
                        )
                        .await
                })
            })
            .await;
        assert!(before_expiry.is_err());
        assert!(
            env.themes()
                .content_eligibility(&digest)
                .await
                .unwrap()
                .is_some()
        );
        let themes = Arc::clone(&env.themes());
        let digest_to_collect = digest.clone();
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .collect_retained_content(
                                transaction,
                                ThemeOwner::Site,
                                &digest_to_collect,
                                10,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(
            env.themes()
                .content_eligibility(&digest)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn owner_and_site_quota_reject_at_exact_boundaries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 10,
            logical_bytes: 4,
            site_retained_revisions: 10,
            site_physical_bytes: 4,
        };
        let site = ThemeOwner::Site;
        let first_author = ThemeOwner::Author(UserId::from(1));
        let second_author = ThemeOwner::Author(UserId::from(2));
        for owner in [site, first_author, second_author] {
            let themes = Arc::clone(&env.themes());
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(
                            async move { themes.admit_theme(transaction, owner, limits).await },
                        )
                    })
                    .await
                    .expect("admit owner at active-theme boundary"),
            );
        }
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move { themes.admit_theme(transaction, site, limits).await })
                })
                .await
                .is_err()
        );

        let first_digest = "d".repeat(64).parse::<ThemeContentDigest>().unwrap();
        let second_digest = "e".repeat(64).parse::<ThemeContentDigest>().unwrap();
        for digest in [&first_digest, &second_digest] {
            let themes = Arc::clone(&env.themes());
            let eligibility = ThemeContentEligibility {
                digest: digest.clone(),
                mime: "image/png".into(),
                retained_until_unix_seconds: 0,
            };
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(async move {
                            themes
                                .upsert_content_eligibility(transaction, &eligibility)
                                .await
                        })
                    })
                    .await
                    .expect("make content eligible"),
            );
        }
        let first_charge = ThemeContentCharge {
            digest: first_digest.clone(),
            logical_bytes: 4,
            physical_bytes: 4,
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(transaction, site, limits, &[first_charge])
                            .await
                    })
                })
                .await
                .expect("admit exact owner and site byte boundary"),
        );
        let duplicate_charge = ThemeContentCharge {
            digest: first_digest,
            logical_bytes: 4,
            physical_bytes: 4,
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(
                                transaction,
                                first_author,
                                limits,
                                &[duplicate_charge],
                            )
                            .await
                    })
                })
                .await
                .expect("same content does not consume site bytes twice"),
        );
        let second_charge = ThemeContentCharge {
            digest: second_digest.clone(),
            logical_bytes: 1,
            physical_bytes: 1,
        };
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(
                                transaction,
                                first_author,
                                limits,
                                &[second_charge],
                            )
                            .await
                    })
                })
                .await
                .is_err()
        );
        let second_charge = ThemeContentCharge {
            digest: second_digest,
            logical_bytes: 1,
            physical_bytes: 1,
        };
        let themes = Arc::clone(&env.themes());
        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(
                                transaction,
                                second_author,
                                limits,
                                &[second_charge],
                            )
                            .await
                    })
                })
                .await
                .is_err()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn competing_owners_can_attach_identical_content(#[case] backend: Backend) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 1,
            logical_bytes: 4,
            site_retained_revisions: 2,
            site_physical_bytes: 4,
        };
        let owners = [ThemeOwner::Site, ThemeOwner::Author(UserId::from(1))];
        for owner in owners {
            let themes = Arc::clone(&env.themes());
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(
                            async move { themes.admit_theme(transaction, owner, limits).await },
                        )
                    })
                    .await
                    .expect("admit competing owner"),
            );
        }
        let digest = "f".repeat(64).parse::<ThemeContentDigest>().unwrap();
        let eligibility = ThemeContentEligibility {
            digest: digest.clone(),
            mime: "image/png".into(),
            retained_until_unix_seconds: 0,
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .upsert_content_eligibility(transaction, &eligibility)
                            .await
                    })
                })
                .await
                .expect("make shared content eligible"),
        );
        let first_themes = Arc::clone(&env.themes());
        let second_themes = Arc::clone(&env.themes());
        let first_scope = env.write_scope().clone();
        let second_scope = env.write_scope().clone();
        let first_charge = ThemeContentCharge {
            digest: digest.clone(),
            logical_bytes: 4,
            physical_bytes: 4,
        };
        let second_charge = first_charge.clone();
        let (first, second) = tokio::join!(
            first_scope.run(move |transaction| {
                Box::pin(async move {
                    first_themes
                        .attach_revision_content(transaction, owners[0], limits, &[first_charge])
                        .await
                })
            }),
            second_scope.run(move |transaction| {
                Box::pin(async move {
                    second_themes
                        .attach_revision_content(transaction, owners[1], limits, &[second_charge])
                        .await
                })
            })
        );
        confirmed(first.expect("first competing attachment"));
        confirmed(second.expect("second competing attachment"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn same_blob_site_physical_charge_is_deduplicated_across_owners(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 1,
            logical_bytes: 4,
            site_retained_revisions: 2,
            site_physical_bytes: 4,
        };
        let owners = [ThemeOwner::Site, ThemeOwner::Author(UserId::from(1))];
        for owner in owners {
            let themes = Arc::clone(&env.themes());
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(
                            async move { themes.admit_theme(transaction, owner, limits).await },
                        )
                    })
                    .await
                    .expect("admit owner"),
            );
        }
        let digest = "a".repeat(64).parse::<ThemeContentDigest>().unwrap();
        let eligibility = ThemeContentEligibility {
            digest: digest.clone(),
            mime: "image/png".into(),
            retained_until_unix_seconds: 0,
        };
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .upsert_content_eligibility(transaction, &eligibility)
                            .await
                    })
                })
                .await
                .expect("make shared content eligible"),
        );
        for owner in owners {
            let themes = Arc::clone(&env.themes());
            let charge = ThemeContentCharge {
                digest: digest.clone(),
                logical_bytes: 4,
                physical_bytes: 4,
            };
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(async move {
                            themes
                                .attach_revision_content(transaction, owner, limits, &[charge])
                                .await
                        })
                    })
                    .await
                    .expect("attach shared content"),
            );
        }
        assert_eq!(
            env.themes().site_quota().await.expect("read site quota"),
            ThemeSiteQuota {
                retained_revisions: 2,
                physical_bytes: 4,
            }
        );
        for owner in owners {
            assert_eq!(
                env.themes()
                    .owner_quota(owner)
                    .await
                    .expect("read owner quota")
                    .expect("owner quota exists")
                    .logical_bytes,
                4
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn catalog_owner_keys_isolate_signed_user_ids_and_site(#[case] backend: Backend) {
        let env = backend.setup().await;
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: b"{}".to_vec(),
            stylesheet: b"body{}".to_vec(),
            source_digest: "b".repeat(64).parse().unwrap(),
            assets: Vec::new(),
        };
        let owners = [
            ThemeOwner::Author(UserId::from(-1)),
            ThemeOwner::Author(UserId::from(0)),
            ThemeOwner::Site,
        ];
        for owner in owners {
            let themes = Arc::clone(&env.themes());
            let draft = draft.clone();
            confirmed(
                env.write_scope()
                    .run(move |transaction| {
                        Box::pin(async move {
                            themes
                                .create_theme(
                                    transaction,
                                    owner,
                                    "Same name",
                                    &draft,
                                    ThemeQuotaLimits {
                                        active_themes: 8,
                                        retained_revisions: 8,
                                        logical_bytes: i64::MAX,
                                        site_retained_revisions: 8,
                                        site_physical_bytes: i64::MAX,
                                    },
                                )
                                .await
                        })
                    })
                    .await
                    .unwrap(),
            );
            assert_eq!(env.themes().list_themes(owner).await.unwrap().len(), 1);
        }
        assert_eq!(
            catalog_owner_key(ThemeOwner::Author(UserId::from(-1))),
            "user:-1"
        );
        assert_eq!(
            catalog_owner_key(ThemeOwner::Author(UserId::from(0))),
            "user:0"
        );
        assert_eq!(catalog_owner_key(ThemeOwner::Site), "site");
    }
    #[apply(backends)]
    #[tokio::test]
    async fn pool_binding_constraint_rejects_a_missing_shuffle_seed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let theme_id = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, i64>(
                "INSERT INTO themes (catalog_owner_key, name, name_key) \
                 VALUES ('site', 'Constraint fixture', 'constraint-fixture') \
                 RETURNING id",
            )
            .fetch_one(pool)
            .await
            .expect("create constraint fixture theme")
        });
        let error = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "INSERT INTO theme_role_bindings \
                 (theme_id, role, mode, pool_revision_digest) \
                 VALUES ($1, 'header', 'pool', $2)",
            )
            .bind(theme_id)
            .bind("a".repeat(64))
            .execute(pool)
            .await
            .expect_err("pool binding without shuffle seed must violate its shape constraint")
        });
        assert!(
            error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_check_violation)
        );
    }

    #[test]
    fn conversion_helpers_reject_malformed_inputs_and_preserve_owner_identity() {
        assert_eq!(
            validate_theme_catalog_name(&"x".repeat(THEME_CATALOG_NAME_MAX_LENGTH + 1)),
            Err(ThemeCatalogNameError::TooLong)
        );
        assert!(
            draft_asset_from_columns(
                Some("asset.png".to_owned()),
                Some("image/png".to_owned()),
                Some(vec![1]),
                Some("invalid".to_owned()),
            )
            .is_none()
        );
        assert!(matches!(
            draft_asset_from_columns(None, None, None, None),
            Some(DraftAssetColumns::Empty)
        ));
        assert_eq!(
            theme_owner_from_key("user:-7"),
            Some(ThemeOwner::Author(UserId::from(-7)))
        );
        assert_eq!(theme_owner_from_key("operator"), None);
        assert!(theme_digest_bytes("not-a-digest").is_err());
    }
    #[test]
    fn column_conversion_rejects_partial_assets_and_author_ids_remain_exact() {
        assert!(
            draft_asset_from_columns(Some("asset.png".to_owned()), None, None, None,).is_none()
        );
        assert_eq!(author_user_id(ThemeOwner::Site), None);
        assert_eq!(
            author_user_id(ThemeOwner::Author(UserId::from(-1))),
            Some(-1)
        );
    }

    #[test]
    fn pool_entry_encoding_requires_exact_relational_shape() {
        let invalid = ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("logo.png".to_owned()),
            media_user_id: Some(UserId::from(1)),
            media_source: None,
            media_digest: None,
            media_filename: None,
        };
        assert!(header_pool_entry_encoding(&invalid).is_err());

        let valid = ThemeHeaderPoolEntry {
            ordinal: 0,

            package_path: None,
            media_user_id: Some(UserId::from(1)),
            media_source: Some("upload".to_owned()),
            media_digest: Some("a".repeat(64).parse().unwrap()),
            media_filename: Some("hero.png".to_owned()),
        };
        let encoded = header_pool_entry_encoding(&valid).expect("complete media entry encodes");
        assert_eq!(encoded[0], 1);
        assert_eq!(encoded.len(), 1 + 8 + 6 + 32 + 8 + 8);
    }
    #[apply(backends)]
    #[tokio::test]
    async fn storage_rejects_invalid_owner_pool_and_charge_shapes(#[case] backend: Backend) {
        let env = backend.setup().await;
        let binding = ThemeRoleBinding::Media {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Logo,
            user_id: UserId::from(2),
            media: MediaRef {
                source: "upload".parse().unwrap(),
                sha256: "a".repeat(64).parse().unwrap(),
                filename: "hero.png".parse().unwrap(),
            },
        };
        let themes = Arc::clone(&env.state.themes);
        assert!(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_role_binding(
                                transaction,
                                ThemeOwner::Author(UserId::from(1)),
                                &binding,
                            )
                            .await
                    })
                })
                .await
                .is_err(),
            "an author binding cannot name another user's media"
        );

        let themes = Arc::clone(&env.state.themes);
        assert!(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .replace_header_pool(
                                transaction,
                                ThemeOwner::Author(UserId::from(1)),
                                ThemeId::from(9),
                                &[ThemeHeaderPoolEntry {
                                    ordinal: 0,
                                    package_path: None,
                                    media_user_id: Some(UserId::from(2)),
                                    media_source: Some("upload".to_owned()),
                                    media_digest: Some("a".repeat(64).parse().unwrap()),
                                    media_filename: Some("hero.png".to_owned()),
                                }],
                            )
                            .await
                    })
                })
                .await
                .is_err(),
            "an author header pool cannot name another user's media"
        );

        let charges = [
            ThemeContentCharge {
                digest: "b".repeat(64).parse().unwrap(),
                logical_bytes: 1,
                physical_bytes: 1,
            },
            ThemeContentCharge {
                digest: "a".repeat(64).parse().unwrap(),
                logical_bytes: 1,
                physical_bytes: 1,
            },
        ];
        let themes = Arc::clone(&env.state.themes);
        assert!(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .detach_revision_content(transaction, ThemeOwner::Site, &charges, 0)
                            .await
                    })
                })
                .await
                .is_err(),
            "content charges must be strictly ordered by digest"
        );
    }

    #[test]
    fn persisted_binding_conversion_accepts_each_complete_shape() {
        let theme_id = ThemeId::from(9);
        let empty = || ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("packaged_default".to_owned()),
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        };
        assert!(matches!(
            binding_from_row(theme_id, ThemeImageRole::Logo, empty()).unwrap(),
            ThemeRoleBinding::PackagedDefault { .. }
        ));
        let mut absent = empty();
        absent.mode = StoredThemeBindingMode("explicit_absent".to_owned());
        assert!(matches!(
            binding_from_row(theme_id, ThemeImageRole::Logo, absent).unwrap(),
            ThemeRoleBinding::ExplicitAbsent { .. }
        ));
        let mut package = empty();
        package.mode = StoredThemeBindingMode("package_asset".to_owned());
        package.package_path = Some(StoredThemePackagePath("images/logo.png".to_owned()));
        assert!(matches!(
            binding_from_row(theme_id, ThemeImageRole::Logo, package).unwrap(),
            ThemeRoleBinding::PackageAsset { package_path, .. } if package_path == "images/logo.png"
        ));
        let media = ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("media".to_owned()),
            package_path: None,
            media_user_id: Some(UserId::from(1)),
            media_source: Some("upload".parse().unwrap()),
            media_digest: Some("a".repeat(64).parse().unwrap()),
            media_filename: Some("hero.png".parse().unwrap()),
            pool_revision: None,
            shuffle_seed: None,
        };
        assert!(matches!(
            binding_from_row(theme_id, ThemeImageRole::Logo, media).unwrap(),
            ThemeRoleBinding::Media { .. }
        ));
        let pool = ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("pool".to_owned()),
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: Some(StoredThemePoolRevision("a".repeat(64))),
            shuffle_seed: Some(StoredThemeShuffleSeed(vec![0; 32])),
        };
        assert!(matches!(
            binding_from_row(theme_id, ThemeImageRole::Header, pool).unwrap(),
            ThemeRoleBinding::HeaderPool { .. }
        ));
    }

    #[test]
    fn persisted_media_and_pool_rows_reject_missing_or_invalid_identity_fields() {
        let incomplete_media = ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("media".to_owned()),
            package_path: None,
            media_user_id: Some(UserId::from(1)),
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        };
        assert!(
            binding_from_row(ThemeId::from(9), ThemeImageRole::Logo, incomplete_media).is_err()
        );
        let invalid_pool_digest = ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("pool".to_owned()),
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: Some(StoredThemePoolRevision("not-a-digest".to_owned())),
            shuffle_seed: Some(StoredThemeShuffleSeed(vec![0; 32])),
        };
        assert!(
            binding_from_row(
                ThemeId::from(9),
                ThemeImageRole::Header,
                invalid_pool_digest
            )
            .is_err()
        );
        let missing_pool_seed = ThemeRoleBindingRow {
            mode: StoredThemeBindingMode("pool".to_owned()),
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: Some(StoredThemePoolRevision("a".repeat(64))),
            shuffle_seed: None,
        };
        assert!(
            binding_from_row(ThemeId::from(9), ThemeImageRole::Header, missing_pool_seed).is_err()
        );
    }

    #[test]
    fn malformed_persisted_binding_columns_fail_conversion() {
        assert!(
            binding_from_row(
                ThemeId::from(9),
                ThemeImageRole::Logo,
                ThemeRoleBindingRow {
                    mode: StoredThemeBindingMode("package_asset".to_owned()),
                    package_path: None,
                    media_user_id: None,
                    media_source: None,
                    media_digest: None,
                    media_filename: None,
                    pool_revision: None,
                    shuffle_seed: None,
                },
            )
            .is_err()
        );
        assert!(
            binding_from_row(
                ThemeId::from(9),
                ThemeImageRole::Logo,
                ThemeRoleBindingRow {
                    mode: StoredThemeBindingMode("pool".to_owned()),
                    package_path: None,
                    media_user_id: None,
                    media_source: None,
                    media_digest: None,
                    media_filename: None,
                    pool_revision: Some(StoredThemePoolRevision("a".repeat(64))),
                    shuffle_seed: Some(StoredThemeShuffleSeed(vec![0; 32])),
                },
            )
            .is_err()
        );
    }
}

impl_theme_storage!(sqlx::Sqlite, write_scope::sqlite_connection);
impl_theme_storage!(sqlx::Postgres, write_scope::postgres_connection);
