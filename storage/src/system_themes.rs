//! Closed system metadata over the existing immutable Theme content lifecycle.
//!
//! Only compiler-minted release inventories can construct an admission. These
//! rows separate system ownership from mutable custom catalogs, not content
//! identity, transaction discipline, filesystem installation, or collection.

use std::collections::{BTreeMap, BTreeSet};

use common::theme::{
    Theme, ThemeContentDigest, ThemeRevisionDigest, ThemeSourceDigest, ThemeStylesheetDigest,
};
use host::system_theme::SystemArtifactInventory;

use crate::{ThemeContentEligibility, ThemePackageAsset};

/// Persisted, immutable metadata for a system-owned bundled revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemThemeRevision {
    pub theme: Theme,
    pub digest: ThemeRevisionDigest,
    pub source_digest: ThemeSourceDigest,
    pub stylesheet_digest: ThemeStylesheetDigest,
    pub manifest: Vec<u8>,
    pub assets: Vec<ThemePackageAsset>,
}

/// System references and the maximum outstanding detached-content guarantee.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemThemeContentReference {
    pub digest: ThemeContentDigest,
    pub live_references: i64,
    pub retained_until_unix_seconds: i64,
}

#[derive(Debug)]
struct SystemContent {
    mime: String,
    references: i64,
}

/// An atomic, closed release admission. There is no public constructor or
/// mutable-field access: custom Theme mutations cannot mint application authority.
#[derive(Debug)]
pub struct SystemThemeAdmission {
    application: ThemeContentDigest,
    application_mime: String,
    revisions: Vec<SystemThemeRevision>,
    contents: BTreeMap<ThemeContentDigest, SystemContent>,
    now_unix_seconds: i64,
    detached_until_unix_seconds: i64,
}

impl SystemThemeAdmission {
    pub(crate) fn from_inventory(
        inventory: &SystemArtifactInventory,
        now_unix_seconds: i64,
    ) -> Result<Self, sqlx::Error> {
        let detached_until_unix_seconds = now_unix_seconds
            .checked_add(crate::THEME_CONTENT_RETENTION_SECONDS)
            .ok_or_else(|| {
                sqlx::Error::Protocol("system content retention deadline overflow".into())
            })?;
        let application = inventory.application().content_digest();
        let application_mime = inventory.application().content().mime().to_owned();
        let mut contents = BTreeMap::from([(
            application.clone(),
            SystemContent {
                mime: application_mime.clone(),
                references: 1,
            },
        )]);
        let mut revisions = Vec::new();
        for package in inventory.themes() {
            // One reference per role and exact digest, including repeated package
            // asset paths that share bytes. Different system roles add references.
            let mut seen = BTreeSet::new();
            for content in package.revision().contents() {
                let digest = ThemeContentDigest::from_digest(content.digest());
                if !seen.insert(digest.clone()) {
                    continue;
                }
                let entry = contents.entry(digest).or_insert_with(|| SystemContent {
                    mime: content.mime().to_owned(),
                    references: 0,
                });
                if entry.mime != content.mime() {
                    return Err(sqlx::Error::Protocol(
                        "system content digest has conflicting MIME types".into(),
                    ));
                }
                entry.references += 1;
            }
            revisions.push(SystemThemeRevision {
                theme: package.theme(),
                digest: package.revision_digest(),
                source_digest: package.source_digest(),
                stylesheet_digest: package.stylesheet_digest(),
                manifest: package.revision().canonical_manifest().to_vec(),
                assets: package
                    .revision()
                    .assets()
                    .map(|(path, mime, _, digest)| ThemePackageAsset {
                        path: path.to_owned(),
                        mime: mime.to_owned(),
                        digest: common::theme::ThemeAssetDigest::from_digest(digest),
                    })
                    .collect(),
            });
        }
        Ok(Self {
            application,
            application_mime,
            revisions,
            contents,
            now_unix_seconds,
            detached_until_unix_seconds,
        })
    }
}

fn parse_digest<T: std::str::FromStr>(value: &str) -> Result<T, sqlx::Error> {
    value
        .parse()
        .map_err(|_| sqlx::Error::Protocol("invalid persisted system content identity".into()))
}

macro_rules! system_queries {
    ($module:ident, $connection:ty, $pool:ty) => {
        pub(crate) mod $module {
            use super::*;

            pub(crate) async fn references(pool: &$pool) -> Result<Vec<SystemThemeContentReference>, sqlx::Error> {
                let rows: Vec<(String, i64, i64)> = sqlx::query_as("SELECT digest, live_references, retained_until_unix_seconds FROM system_theme_content_references ORDER BY digest").fetch_all(pool).await?;
                rows.into_iter().map(|(digest, live_references, retained_until_unix_seconds)| Ok(SystemThemeContentReference { digest: parse_digest(&digest)?, live_references, retained_until_unix_seconds })).collect()
            }

            /// Reads only current roles through the partial live-reference index.
            /// Detached history is deliberately excluded from installation locks.
            pub(crate) async fn live_references(pool: &$pool) -> Result<Vec<SystemThemeContentReference>, sqlx::Error> {
                let rows: Vec<(String, i64, i64)> = sqlx::query_as("SELECT digest, live_references, retained_until_unix_seconds FROM system_theme_content_references WHERE live_references > 0 ORDER BY digest").fetch_all(pool).await?;
                rows.into_iter().map(|(digest, live_references, retained_until_unix_seconds)| Ok(SystemThemeContentReference { digest: parse_digest(&digest)?, live_references, retained_until_unix_seconds })).collect()
            }

            pub(crate) async fn application(pool: &$pool) -> Result<Option<ThemeContentEligibility>, sqlx::Error> {
                let row: Option<(String, String, i64)> = sqlx::query_as("SELECT a.digest, a.mime, e.retained_until_unix_seconds FROM system_application_current a JOIN theme_content_eligibility e ON e.digest = a.digest WHERE a.singleton = 1").fetch_optional(pool).await?;
                row.map(|(digest, mime, retained_until_unix_seconds)| Ok(ThemeContentEligibility { digest: parse_digest(&digest)?, mime, retained_until_unix_seconds })).transpose()
            }

            pub(crate) async fn revision(pool: &$pool, theme: Theme) -> Result<Option<SystemThemeRevision>, sqlx::Error> {
                let row: Option<(String, String, String, Vec<u8>)> = sqlx::query_as("SELECT r.revision_digest, r.source_digest, r.stylesheet_digest, r.manifest FROM system_theme_current c JOIN system_theme_revisions r ON r.theme_token = c.theme_token AND r.revision_digest = c.revision_digest WHERE c.theme_token = $1").bind(theme.token()).fetch_optional(pool).await?;
                let Some((digest, source_digest, stylesheet_digest, manifest)) = row else { return Ok(None); };
                let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT path, digest, mime FROM system_theme_revision_assets WHERE theme_token = $1 AND revision_digest = $2 ORDER BY path").bind(theme.token()).bind(&digest).fetch_all(pool).await?;
                let assets = rows.into_iter().map(|(path, digest, mime)| Ok(ThemePackageAsset { path, digest: parse_digest(&digest)?, mime })).collect::<Result<Vec<_>, sqlx::Error>>()?;
                Ok(Some(SystemThemeRevision { theme, digest: parse_digest(&digest)?, source_digest: parse_digest(&source_digest)?, stylesheet_digest: parse_digest(&stylesheet_digest)?, manifest, assets }))
            }

            pub(crate) async fn admit(connection: &mut $connection, admission: &SystemThemeAdmission) -> Result<(), sqlx::Error> {
                // The existing site admission lock serializes system, custom, and
                // collection SQL without charging system content to custom quotas.
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                // Admission touches only current roles and this closed incoming
                // inventory. Detached history is not release state: querying it
                // after this first write would make SQLite lock occupancy grow
                // with every old release. Incoming lookups retain a prior zero
                // reference deadline for rollback/reattachment semantics.
                let live: Vec<(String, i64, i64)> = sqlx::query_as("SELECT digest, live_references, retained_until_unix_seconds FROM system_theme_content_references WHERE live_references > 0 ORDER BY digest").fetch_all(&mut *connection).await?;
                let mut previous = live.into_iter().map(|(digest, references, deadline)| Ok((parse_digest::<ThemeContentDigest>(&digest)?, (references, deadline)))).collect::<Result<BTreeMap<_, _>, sqlx::Error>>()?;
                for digest in admission.contents.keys() {
                    let row: Option<(i64, i64)> = sqlx::query_as("SELECT live_references, retained_until_unix_seconds FROM system_theme_content_references WHERE digest = $1").bind(digest.as_ref()).fetch_optional(&mut *connection).await?;
                    if let Some((references, deadline)) = row {
                        previous.insert(digest.clone(), (references, deadline));
                    }
                }
                let digests = previous.keys().chain(admission.contents.keys()).cloned().collect::<BTreeSet<_>>();
                for digest in &digests {
                    if let Some(content) = admission.contents.get(digest) {
                        sqlx::query("INSERT INTO theme_content_eligibility (digest, mime, retained_until_unix_seconds) VALUES ($1, $2, $3) ON CONFLICT (digest) DO NOTHING").bind(digest.as_ref()).bind(&content.mime).bind(admission.now_unix_seconds).execute(&mut *connection).await?;
                    }
                    sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(digest.as_ref()).execute(&mut *connection).await?;
                }
                for digest in &digests {
                    let (old_count, old_deadline) = previous.get(digest).copied().unwrap_or((0, admission.now_unix_seconds));
                    let content = admission.contents.get(digest);
                    let new_count = content.map_or(0, |content| content.references);
                    if let Some(content) = content {
                        let (mime,): (String,) = sqlx::query_as("SELECT mime FROM theme_content_eligibility WHERE digest = $1").bind(digest.as_ref()).fetch_one(&mut *connection).await?;
                        if mime != content.mime { return Err(sqlx::Error::Protocol("system content conflicts with existing serving MIME".into())); }
                    }
                    if old_count == new_count { continue; }
                    let delta = new_count - old_count;
                    let deadline = if delta < 0 { old_deadline.max(admission.detached_until_unix_seconds) } else { old_deadline };
                    // Reattachment changes only counts. Each subsequent release
                    // extends (never resets/shortens) the exact-byte guarantee.
                    let changed = if delta < 0 {
                        sqlx::query("UPDATE theme_content_eligibility SET live_references = live_references + $1, retained_until_unix_seconds = CASE WHEN retained_until_unix_seconds < $2 THEN $2 ELSE retained_until_unix_seconds END WHERE digest = $3 AND live_references >= $4").bind(delta).bind(deadline).bind(digest.as_ref()).bind(-delta).execute(&mut *connection).await?
                    } else {
                        sqlx::query("UPDATE theme_content_eligibility SET live_references = live_references + $1 WHERE digest = $2").bind(delta).bind(digest.as_ref()).execute(&mut *connection).await?
                    };
                    if changed.rows_affected() != 1 { return Err(sqlx::Error::RowNotFound); }
                    sqlx::query("INSERT INTO system_theme_content_references (digest, live_references, retained_until_unix_seconds) VALUES ($1, $2, $3) ON CONFLICT (digest) DO UPDATE SET live_references = excluded.live_references, retained_until_unix_seconds = excluded.retained_until_unix_seconds").bind(digest.as_ref()).bind(new_count).bind(deadline).execute(&mut *connection).await?;
                }
                for revision in &admission.revisions {
                    sqlx::query("INSERT INTO system_theme_revisions (theme_token, revision_digest, source_digest, stylesheet_digest, manifest) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (theme_token, revision_digest) DO NOTHING").bind(revision.theme.token()).bind(revision.digest.as_ref()).bind(revision.source_digest.as_ref()).bind(revision.stylesheet_digest.as_ref()).bind(&revision.manifest).execute(&mut *connection).await?;
                    for asset in &revision.assets {
                        sqlx::query("INSERT INTO system_theme_revision_assets (theme_token, revision_digest, path, digest, mime) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (theme_token, revision_digest, path) DO NOTHING").bind(revision.theme.token()).bind(revision.digest.as_ref()).bind(&asset.path).bind(asset.digest.as_ref()).bind(&asset.mime).execute(&mut *connection).await?;
                    }
                    sqlx::query("INSERT INTO system_theme_current (theme_token, revision_digest) VALUES ($1, $2) ON CONFLICT (theme_token) DO UPDATE SET revision_digest = excluded.revision_digest").bind(revision.theme.token()).bind(revision.digest.as_ref()).execute(&mut *connection).await?;
                }
                sqlx::query("INSERT INTO system_application_current (singleton, digest, mime) VALUES (1, $1, $2) ON CONFLICT (singleton) DO UPDATE SET digest = excluded.digest, mime = excluded.mime").bind(admission.application.as_ref()).bind(&admission.application_mime).execute(&mut *connection).await?;
                Ok(())
            }

            pub(crate) async fn expired(pool: &$pool, now: i64) -> Result<Vec<ThemeContentDigest>, sqlx::Error> {
                let rows: Vec<(String,)> = sqlx::query_as("SELECT s.digest FROM system_theme_content_references s JOIN theme_content_eligibility e ON e.digest = s.digest WHERE s.live_references = 0 AND s.retained_until_unix_seconds <= $1 AND e.live_references = 0 AND e.retained_until_unix_seconds <= $1 ORDER BY s.digest").bind(now).fetch_all(pool).await?;
                rows.into_iter().map(|(digest,)| parse_digest(&digest)).collect()
            }

            pub(crate) async fn collect(connection: &mut $connection, digest: &ThemeContentDigest, now: i64) -> Result<(), sqlx::Error> {
                sqlx::query("UPDATE theme_site_quota SET retained_revisions = retained_revisions WHERE singleton = 1").execute(&mut *connection).await?;
                sqlx::query("UPDATE theme_content_eligibility SET digest = digest WHERE digest = $1").bind(digest.as_ref()).execute(&mut *connection).await?;
                let (eligible,): (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM system_theme_content_references s JOIN theme_content_eligibility e ON e.digest = s.digest WHERE s.digest = $1 AND s.live_references = 0 AND s.retained_until_unix_seconds <= $2 AND e.live_references = 0 AND e.retained_until_unix_seconds <= $2 AND NOT EXISTS (SELECT 1 FROM theme_retained_content_charges c WHERE c.digest = e.digest))").bind(digest.as_ref()).bind(now).fetch_one(&mut *connection).await?;
                if !eligible { return Err(sqlx::Error::RowNotFound); }
                sqlx::query("DELETE FROM system_theme_content_references WHERE digest = $1").bind(digest.as_ref()).execute(&mut *connection).await?;
                sqlx::query("DELETE FROM theme_content_eligibility WHERE digest = $1 AND live_references = 0 AND retained_until_unix_seconds <= $2").bind(digest.as_ref()).bind(now).execute(&mut *connection).await?;
                Ok(())
            }
        }
    };
}

system_queries!(sqlite, sqlx::SqliteConnection, sqlx::SqlitePool);
system_queries!(postgres, sqlx::PgConnection, sqlx::PgPool);
