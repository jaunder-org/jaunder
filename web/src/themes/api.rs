//! Theme catalog management server functions and their wire DTOs.

use common::{
    MutationOutcome,
    ids::ThemeId,
    media::{ContentHash, Filename, MediaSource},
    theme::{PublicThemeSelection, ThemeImageRole},
};
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use serde::{Deserialize, Serialize};

use crate::error::WebResult;

/// A catalog scope selected by the authenticated principal, never by a supplied user ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipScope {
    Site,
    Author,
}

/// A catalog row safe to return to its owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: ThemeId,
    pub name: String,
    pub published: bool,
}

/// Editable package source. The authenticated owner receives it only with no-store caching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub manifest: Vec<u8>,
    pub stylesheet: Vec<u8>,
    #[serde(default)]
    pub assets: Vec<ThemeAssetInput>,
}

/// One portable package asset. Runtime Media bindings are deliberately not part of this value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeAssetInput {
    pub path: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A portable package archive and a safe attachment filename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedPackage {
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// An isolated semantic preview: transformed CSS accompanies real Style Contract markup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePreview {
    pub html: String,
    pub css: String,
}
/// A wire-safe Media reference. Ownership is derived from the authenticated principal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeMediaInput {
    pub source: MediaSource,
    pub sha256: ContentHash,
    pub filename: Filename,
}

/// Caller-selected source for one fixed logo/header binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ThemeBindingInput {
    PackagedDefault,
    ExplicitAbsent,
    PackageAsset(String),
    Media(ThemeMediaInput),
}

/// Caller-selected source for one explicit header-pool member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ThemePoolInput {
    PackageAsset(String),
    Media(ThemeMediaInput),
}
/// Owner-visible presentation state for one draft, without persisted Media owner IDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePresentation {
    pub logo: Option<ThemeBindingInput>,
    pub header: Option<ThemeBindingInput>,
    pub header_pool: Vec<ThemePoolInput>,
    pub shuffle_seed: Option<[u8; 32]>,
}

#[cfg(feature = "server")]
use {
    crate::{
        auth,
        error::{self, InternalError},
    },
    common::{media::MediaRef, theme},
    host::{
        theme_operations::{ThemeOperationCoordinator, ThemeOperationRejected},
        theme_package::{self, ThemePackageLimits, ValidatedThemePackage},
    },
    jiff::Timestamp,
    leptos::prelude::*,
    leptos_axum::ResponseOptions,
    std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    },
    storage::{
        PostStorage, ReplaceDraftError, ThemeAssetError, ThemeAssetManager, ThemeDraft,
        ThemeDraftAsset, ThemeManager, ThemeOwner, ThemeQuotaLimits, ThemeRoleInput, ThemeStorage,
        UserStorage, WriteScope,
    },
};
#[cfg(feature = "server")]
fn plain_css_manifest(name: &str) -> Result<Vec<u8>, InternalError> {
    serde_json::to_vec(&serde_json::json!({
        "assets": {},
        "defaults": {},
        "name": name,
        "schema": 1,
        "style_contract": theme::STYLE_CONTRACT_VERSION,
    }))
    .map_err(InternalError::external)
}
#[cfg(feature = "server")]
fn digest_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = std::fmt::Write::write_fmt(&mut value, format_args!("{byte:02x}"));
    }
    value
}

#[cfg(feature = "server")]
fn no_store() {
    if let Some(options) = use_context::<ResponseOptions>() {
        options.insert_header(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("private, no-store"),
        );
    }
}

#[cfg(feature = "server")]
async fn owner(scope: OwnershipScope) -> Result<(common::ids::UserId, ThemeOwner), InternalError> {
    let actor = auth::require_auth().await?;
    match scope {
        OwnershipScope::Author => Ok((actor.user_id, ThemeOwner::Author(actor.user_id))),
        OwnershipScope::Site => {
            auth::require_operator().await?;
            Ok((actor.user_id, ThemeOwner::Site))
        }
    }
}

#[cfg(feature = "server")]
fn admission_error(error: ThemeOperationRejected) -> InternalError {
    match error {
        ThemeOperationRejected::RateLimited => {
            InternalError::validation("theme operation rate limited")
        }
        ThemeOperationRejected::InFlight => {
            InternalError::validation("theme operation already in progress")
        }
    }
}
#[cfg(feature = "server")]
// `multer::Error` is constructed only by Axum's streaming multipart decoder.
// cov:ignore-start
fn multipart_error(error: multer::Error) -> InternalError {
    InternalError::validation_source("invalid multipart theme import", error)
}
// cov:ignore-stop

#[cfg(feature = "server")]
fn catalog(entry: storage::ThemeCatalogEntry) -> CatalogEntry {
    CatalogEntry {
        id: entry.id,
        name: entry.name,
        published: entry.current_revision.is_some(),
    }
}

#[cfg(feature = "server")]
fn draft_from_input(theme_id: ThemeId, input: Draft) -> Result<ThemeDraft, InternalError> {
    let mut paths = BTreeSet::new();
    for asset in &input.assets {
        if !paths.insert(&asset.path) {
            return Err(InternalError::validation(format!(
                "duplicate theme asset path: {}",
                asset.path
            )));
        }
    }
    let archive = theme_package::export_theme_package(
        &input.manifest,
        &input.stylesheet,
        &input
            .assets
            .iter()
            .map(|asset| (asset.path.clone(), asset.bytes.clone()))
            .collect(),
    )
    .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let package = theme_package::validate_theme_package(&archive, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let assets = input
        .assets
        .into_iter()
        .map(|asset| {
            let (mime, _, digest) = package
                .asset(&asset.path)
                .ok_or_else(|| InternalError::validation("package asset disappeared"))?;
            Ok::<_, InternalError>(ThemeDraftAsset {
                path: asset.path,
                mime: mime.to_owned(),
                bytes: asset.bytes,
                digest: digest_hex(&digest)
                    .parse()
                    .map_err(|_| InternalError::validation("invalid package digest"))?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = package.canonical_manifest().to_vec();
    let stylesheet = package.authored_css().to_vec();
    let source_digest = digest_hex(&package.source_digest())
        .parse()
        .map_err(|_| InternalError::validation("invalid package digest"))?;
    validate_draft_css(package)?;
    Ok(ThemeDraft {
        theme_id,
        manifest,
        stylesheet,
        source_digest,
        assets,
    })
}

#[cfg(feature = "server")]
fn draft_from_archive(theme_id: ThemeId, archive: &[u8]) -> Result<ThemeDraft, InternalError> {
    let package = theme_package::validate_theme_package(archive, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let assets = package
        .asset_paths()
        .map(|path| {
            let (mime, bytes, digest) = package
                .asset(path)
                .ok_or_else(|| InternalError::server_message("validated asset disappeared"))?;
            Ok(ThemeDraftAsset {
                path: path.to_owned(),
                mime: mime.to_owned(),
                bytes: bytes.to_vec(),
                digest: digest_hex(&digest)
                    .parse()
                    .map_err(|_| InternalError::server_message("validated asset digest"))?,
            })
        })
        .collect::<Result<Vec<_>, InternalError>>()?;
    let manifest = package.canonical_manifest().to_vec();
    let stylesheet = package.authored_css().to_vec();
    let source_digest = digest_hex(&package.source_digest())
        .parse()
        .map_err(|_| InternalError::server_message("validated source digest"))?;
    validate_draft_css(package)?;
    Ok(ThemeDraft {
        theme_id,
        manifest,
        stylesheet,
        source_digest,
        assets,
    })
}
#[cfg(feature = "server")]
fn package_asset_urls(
    package: &ValidatedThemePackage,
) -> Result<BTreeMap<String, String>, InternalError> {
    package
        .asset_paths()
        .map(|path| {
            let (_, _, digest) = package
                .asset(path)
                .ok_or_else(|| InternalError::server_message("validated asset disappeared"))?;
            Ok((path.to_owned(), format!("/theme/{}", digest_hex(&digest))))
        })
        .collect()
}

#[cfg(feature = "server")]
fn validate_draft_css(package: ValidatedThemePackage) -> Result<(), InternalError> {
    let asset_urls = package_asset_urls(&package)?;
    package
        .compile(&asset_urls, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    Ok(())
}

#[cfg(feature = "server")]
fn draft_asset_urls(
    package: &ValidatedThemePackage,
    theme_id: ThemeId,
) -> BTreeMap<String, String> {
    package
        .asset_paths()
        .map(|path| {
            (
                path.to_owned(),
                format!(
                    "/theme/draft/{theme_id}/{}",
                    percent_encode_draft_asset_path(path)
                ),
            )
        })
        .collect()
}

#[cfg(feature = "server")]
fn percent_encode_draft_asset_path(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/')
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

// Storage error variants are projected defensively; backend-specific variants require an
// actual database driver to construct and are exercised by the storage integration suite.
// cov:ignore-start
#[cfg(feature = "server")]
fn create_storage_error(error: sqlx::Error) -> InternalError {
    match &error {
        sqlx::Error::RowNotFound => InternalError::validation_source("theme quota exceeded", error),
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            InternalError::conflict("theme name already exists")
        }
        _ => InternalError::storage(error),
    }
}

#[cfg(feature = "server")]
fn theme_storage_error(error: sqlx::Error) -> InternalError {
    match &error {
        sqlx::Error::RowNotFound => InternalError::not_found("theme"),
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            InternalError::conflict("theme name already exists")
        }
        _ => InternalError::storage(error),
    }
}
#[cfg(feature = "server")]
fn replace_draft_error(error: ReplaceDraftError) -> InternalError {
    match error {
        ReplaceDraftError::OwnerNotFound => InternalError::not_found("theme"),
        ReplaceDraftError::QuotaExceeded => InternalError::validation("theme draft quota exceeded"),
        ReplaceDraftError::Storage(error) => InternalError::storage(error),
    }
}
// cov:ignore-stop

#[cfg(feature = "server")]
fn theme_name(name: &str) -> Result<String, InternalError> {
    storage::validate_theme_catalog_name(name)
        .map_err(|error| InternalError::validation(error.to_string()))
}

#[cfg(feature = "server")]
fn manager_error(error: anyhow::Error) -> InternalError {
    if matches!(
        error.downcast_ref::<sqlx::Error>(),
        Some(sqlx::Error::RowNotFound)
    ) {
        return InternalError::not_found("theme");
    }
    InternalError::server_boxed(error.into_boxed_dyn_error())
}

#[cfg(feature = "server")]
fn publication_error(error: ThemeAssetError) -> InternalError {
    if matches!(&error, ThemeAssetError::Storage(sqlx::Error::RowNotFound)) {
        InternalError::validation_source("theme publication rejected", error)
    } else {
        InternalError::server(error)
    }
}

#[cfg(feature = "server")]
async fn ensure_owned(owner: ThemeOwner, theme_id: ThemeId) -> Result<(), InternalError> {
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .map(|_| ())
        .ok_or_else(|| InternalError::not_found("theme"))
}

#[cfg(feature = "server")]
fn draft_wire(draft: ThemeDraft) -> Draft {
    Draft {
        manifest: draft.manifest,
        stylesheet: draft.stylesheet,
        assets: draft
            .assets
            .into_iter()
            .map(|asset| ThemeAssetInput {
                path: asset.path,
                mime: asset.mime,
                bytes: asset.bytes,
            })
            .collect(),
    }
}
#[cfg(feature = "server")]
fn media_wire(
    source: Option<String>,
    digest: Option<common::theme::ThemeContentDigest>,
    filename: Option<String>,
) -> Result<ThemeMediaInput, InternalError> {
    Ok(ThemeMediaInput {
        source: source
            .ok_or_else(|| InternalError::server_message("missing bound Media source"))?
            .parse()
            .map_err(|_| InternalError::server_message("invalid bound Media source"))?,
        sha256: digest
            .ok_or_else(|| InternalError::server_message("missing bound Media digest"))?
            .as_ref()
            .parse()
            .map_err(|_| InternalError::server_message("invalid bound Media digest"))?,
        filename: filename
            .ok_or_else(|| InternalError::server_message("missing bound Media filename"))?
            .parse()
            .map_err(|_| InternalError::server_message("invalid bound Media filename"))?,
    })
}

#[cfg(feature = "server")]
fn binding_wire(binding: storage::ThemeRoleBinding) -> Result<ThemeBindingInput, InternalError> {
    match binding {
        storage::ThemeRoleBinding::PackagedDefault { .. } => Ok(ThemeBindingInput::PackagedDefault),
        storage::ThemeRoleBinding::ExplicitAbsent { .. } => Ok(ThemeBindingInput::ExplicitAbsent),
        storage::ThemeRoleBinding::PackageAsset { package_path, .. } => {
            Ok(ThemeBindingInput::PackageAsset(package_path))
        }
        storage::ThemeRoleBinding::Media { media, .. } => {
            Ok(ThemeBindingInput::Media(ThemeMediaInput {
                source: media.source,
                sha256: media.sha256,
                filename: media.filename,
            }))
        }
        storage::ThemeRoleBinding::HeaderPool { .. } => Err(InternalError::server_message(
            "header pool binding cannot be read as a fixed binding",
        )),
    }
}

#[cfg(feature = "server")]
fn pool_wire(entry: storage::ThemeHeaderPoolEntry) -> Result<ThemePoolInput, InternalError> {
    match entry.package_path {
        Some(path) => Ok(ThemePoolInput::PackageAsset(path)),
        None => Ok(ThemePoolInput::Media(media_wire(
            entry.media_source,
            entry.media_digest,
            entry.media_filename,
        )?)),
    }
}

/// Lists one catalog after authenticating and authorizing its scope.
#[macros::server(skip_all)]
pub async fn list(scope: OwnershipScope) -> WebResult<Vec<CatalogEntry>> {
    let (_, owner) = owner(scope).await?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();

    no_store();
    themes
        .list_themes(owner)
        .await
        .map(|entries| entries.into_iter().map(catalog).collect())
        .map_err(InternalError::storage)
}

/// Reads an owned editable draft; foreign and missing IDs are both masked as not found.
#[macros::server(skip_all)]
pub async fn get_draft(scope: OwnershipScope, theme_id: ThemeId) -> WebResult<Draft> {
    let (_, owner) = owner(scope).await?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    no_store();
    themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .map(draft_wire)
        .ok_or_else(|| InternalError::not_found("theme"))
}

/// Reads all owner-visible presentation bindings for one draft without Media owner IDs.
#[macros::server(skip_all)]
pub async fn get_presentation(
    scope: OwnershipScope,
    theme_id: ThemeId,
) -> WebResult<ThemePresentation> {
    let (_, owner) = owner(scope).await?;
    no_store();
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    ensure_owned(owner, theme_id).await?;
    let logo = themes
        .role_binding(owner, theme_id, ThemeImageRole::Logo)
        .await
        .map_err(InternalError::storage)?
        .map(binding_wire)
        .transpose()?;
    let header_binding = themes
        .role_binding(owner, theme_id, ThemeImageRole::Header)
        .await
        .map_err(InternalError::storage)?;
    // Header pools are represented separately on the wire; fixed bindings deliberately omit them.
    // cov:ignore-start
    let shuffle_seed = match &header_binding {
        Some(storage::ThemeRoleBinding::HeaderPool { shuffle_seed, .. }) => Some(*shuffle_seed),
        _ => None,
    };
    let header = match header_binding {
        Some(storage::ThemeRoleBinding::HeaderPool { .. }) => None,
        binding => binding.map(binding_wire).transpose()?,
    };
    // cov:ignore-stop
    let header_pool = themes
        .header_pool(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .into_iter()
        .map(pool_wire)
        .collect::<Result<_, _>>()?;
    Ok(ThemePresentation {
        logo,
        header,
        header_pool,
        shuffle_seed,
    })
}

/// Reads a site's or authenticated author's explicit selection; `None` for an
/// author means inherit the site presentation.
#[macros::server(skip_all)]
pub async fn get_selection(scope: OwnershipScope) -> WebResult<Option<PublicThemeSelection>> {
    let (_, owner) = owner(scope).await?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    no_store();
    themes
        .selection(owner)
        .await
        .map_err(InternalError::storage)
}

/// Creates a catalog entry with a validated portable package draft.
#[macros::server(skip_all)]
pub async fn create(
    scope: OwnershipScope,
    name: String,
    draft: Draft,
) -> WebResult<MutationOutcome<CatalogEntry>> {
    no_store();
    let (actor, owner) = owner(scope).await?;
    let name = theme_name(&name)?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    let draft = draft_from_input(ThemeId::from(0), draft)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let name_for_write = name.clone();
    let draft_for_write = draft.clone();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .create_theme(
                        transaction,
                        owner,
                        &name_for_write,
                        &draft_for_write,
                        ThemeQuotaLimits::production(),
                    )
                    .await
                    .map_err(create_storage_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)?;
    Ok(outcome.map(|id| CatalogEntry {
        id,
        name,
        published: false,
    }))
}

/// Replaces an existing draft from a complete plain-CSS/package editor payload.
#[macros::server(skip_all)]
pub async fn import_package(
    scope: OwnershipScope,
    theme_id: ThemeId,
    draft: Draft,
) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    ensure_owned(owner, theme_id).await?;
    let draft = draft_from_input(theme_id, draft)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .replace_draft(transaction, owner, &draft, ThemeQuotaLimits::production())
                    .await
                    .map_err(replace_draft_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)?;
    Ok(outcome)
}

/// Imports a complete ZIP Theme Package as a new, unselected private draft.
///
/// The ordered multipart fields are `scope`, `name`, then `archive`; principal
/// admission and site authorization happen before archive bytes are consumed.
#[macros::server(input = MultipartFormData, skip_all)]
pub async fn import_zip(data: MultipartData) -> WebResult<MutationOutcome<CatalogEntry>> {
    let actor = auth::require_auth().await?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator
        .acquire(actor.user_id)
        .map_err(admission_error)?;
    let mut multipart = data
        .into_inner()
        .ok_or_else(|| InternalError::validation("missing multipart body"))?;
    let scope = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| InternalError::validation("missing theme ownership scope"))?;
    // Multipart field ordering is enforced at the Axum streaming boundary.
    // cov:ignore-start
    if scope.name() != Some("scope") {
        return Err(InternalError::validation(
            "theme import fields must be scope, name, archive",
        ));
    }
    let owner = match scope.text().await.map_err(multipart_error)?.as_str() {
        "site" => {
            auth::require_operator().await?;
            ThemeOwner::Site
        }
        "author" => ThemeOwner::Author(actor.user_id),
        _ => return Err(InternalError::validation("invalid theme ownership scope")),
    };
    // cov:ignore-stop
    let name = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| InternalError::validation("missing theme name"))?;
    // cov:ignore-start
    if name.name() != Some("name") {
        return Err(InternalError::validation(
            "theme import fields must be scope, name, archive",
        ));
    }
    let name = theme_name(&name.text().await.map_err(multipart_error)?)?;
    let mut archive = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| InternalError::validation("missing theme archive"))?;
    if archive.name() != Some("archive") {
        return Err(InternalError::validation(
            "theme import fields must be scope, name, archive",
        ));
    }
    // cov:ignore-stop
    let limit = ThemePackageLimits::default().max_archive_bytes;
    let mut bytes = Vec::new();
    while let Some(chunk) = archive.chunk().await.map_err(multipart_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(InternalError::validation("theme archive is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    drop(archive);
    // cov:ignore-start
    if multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .is_some()
    {
        return Err(InternalError::validation(
            "theme import fields must be scope, name, archive",
        ));
    }
    // cov:ignore-stop
    let draft = draft_from_archive(ThemeId::from(0), &bytes)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let name_for_write = name.clone();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .create_theme(
                        transaction,
                        owner,
                        &name_for_write,
                        &draft,
                        ThemeQuotaLimits::production(),
                    )
                    .await
                    .map_err(create_storage_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)?;
    no_store();
    Ok(outcome.map(|id| CatalogEntry {
        id,
        name,
        published: false,
    }))
}

/// Imports plain CSS as a new private version-1 package draft with no assets.
#[macros::server(skip_all)]
pub async fn import_css(
    scope: OwnershipScope,
    name: String,
    stylesheet: Vec<u8>,
) -> WebResult<MutationOutcome<CatalogEntry>> {
    let (actor, owner) = owner(scope).await?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    let name = theme_name(&name)?;
    if stylesheet.len() > ThemePackageLimits::default().max_file_bytes {
        return Err(InternalError::validation("theme stylesheet is too large"));
    }
    let draft = draft_from_input(
        ThemeId::from(0),
        Draft {
            manifest: plain_css_manifest(&name)?,
            stylesheet,
            assets: Vec::new(),
        },
    )?; // cov:ignore — successful import exercises validation; this is compiler `?` bookkeeping
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let name_for_write = name.clone();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .create_theme(
                        transaction,
                        owner,
                        &name_for_write,
                        &draft,
                        ThemeQuotaLimits::production(),
                    )
                    .await
                    .map_err(create_storage_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)?;
    no_store();
    Ok(outcome.map(|id| CatalogEntry {
        id,
        name,
        published: false,
    }))
}

/// Replaces stylesheet source while retaining the existing manifest and assets.
#[macros::server(skip_all)]
pub async fn replace_css(
    scope: OwnershipScope,
    theme_id: ThemeId,
    stylesheet: Vec<u8>,
) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let draft = themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .ok_or_else(|| InternalError::not_found("theme"))?;
    let mut draft = draft_wire(draft);
    draft.stylesheet = stylesheet;
    let draft = draft_from_input(theme_id, draft)?;
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .replace_draft(transaction, owner, &draft, ThemeQuotaLimits::production())
                    .await
                    .map_err(replace_draft_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)
}

/// Exports the exact owned draft package without relational Media bindings.
#[macros::server(skip_all)]
pub async fn export(scope: OwnershipScope, theme_id: ThemeId) -> WebResult<ExportedPackage> {
    let (_, owner) = owner(scope).await?;
    no_store();
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let entry = themes
        .list_themes(owner)
        .await
        .map_err(InternalError::storage)?
        .into_iter()
        .find(|entry| entry.id == theme_id)
        .ok_or_else(|| InternalError::not_found("theme"))?;
    let draft = themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .ok_or_else(|| InternalError::not_found("theme"))?;
    let assets = draft
        .assets
        .iter()
        .map(|asset| (asset.path.clone(), asset.bytes.clone()))
        .collect();
    let bytes = theme_package::export_theme_package(&draft.manifest, &draft.stylesheet, &assets)
        .map_err(InternalError::server)?;
    let filename = safe_filename(&entry.name);
    // Response headers are observable only through the Axum HTTP adapter.
    // cov:ignore-start
    if let Some(options) = use_context::<ResponseOptions>() {
        options.insert_header(
            axum::http::header::CONTENT_DISPOSITION,
            axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .map_err(InternalError::server)?,
        );
    }
    // cov:ignore-stop
    Ok(ExportedPackage { filename, bytes })
}

#[cfg(feature = "server")]
fn safe_filename(name: &str) -> String {
    let stem: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(64)
        .collect();
    format!("{}.zip", if stem.is_empty() { "theme" } else { &stem })
}

/// Renames an owned catalog entry.
#[macros::server(skip_all)]
pub async fn rename(
    scope: OwnershipScope,
    theme_id: ThemeId,
    name: String,
) -> WebResult<MutationOutcome<()>> {
    let (_, owner) = owner(scope).await?;
    let name = theme_name(&name)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .rename_theme(transaction, owner, theme_id, &name)
                    .await
                    .map_err(theme_storage_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)
}

/// Removes an owned catalog atomically with its Media bindings.
#[macros::server(skip_all)]
pub async fn remove(scope: OwnershipScope, theme_id: ThemeId) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    ensure_owned(owner, theme_id).await?;
    let manager = expect_context::<Arc<ThemeManager>>();
    manager
        .remove_theme(
            actor,
            owner,
            theme_id,
            Timestamp::now().as_second() + storage::THEME_CONTENT_RETENTION_SECONDS,
        )
        .await
        .map_err(manager_error)
}

/// Publishes the complete owned draft through the host compiler and immutable asset manager.
#[macros::server(skip_all)]
pub async fn publish(scope: OwnershipScope, theme_id: ThemeId) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let draft = themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .ok_or_else(|| InternalError::not_found("theme"))?;
    let archive = theme_package::export_theme_package(
        &draft.manifest,
        &draft.stylesheet,
        &draft
            .assets
            .iter()
            .map(|asset| (asset.path.clone(), asset.bytes.clone()))
            .collect(),
    )
    .map_err(InternalError::server)?;
    let package = theme_package::validate_theme_package(&archive, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let urls = package_asset_urls(&package)?;
    let compiled = package
        .compile(&urls, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let manager = expect_context::<Arc<ThemeAssetManager>>();
    manager
        .publish(
            owner,
            theme_id,
            &compiled,
            ThemeQuotaLimits::production(),
            Timestamp::now().as_second(),
        )
        .await
        .map(|outcome| outcome.map(|_| ()))
        .map_err(publication_error)
}

/// Selects a built-in or owned custom theme; an author may clear selection to inherit site presentation.
#[macros::server(skip_all)]
pub async fn select(
    scope: OwnershipScope,
    selection: Option<PublicThemeSelection>,
) -> WebResult<MutationOutcome<()>> {
    let (_, owner) = owner(scope).await?;
    if let Some(PublicThemeSelection::Custom(theme_id)) = selection {
        let themes = expect_context::<Arc<dyn ThemeStorage>>();
        // Selection validation is an owner-visible catalog read; the absence projection is
        // covered by the HTTP authorization integration path.
        // cov:ignore-start
        if !themes
            .list_themes(owner)
            .await
            .map_err(InternalError::storage)?
            .into_iter()
            .any(|entry| entry.id == theme_id && entry.current_revision.is_some())
        {
            return Err(InternalError::not_found("theme"));
        }
        // cov:ignore-stop
    }
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .set_selection(transaction, owner, selection)
                    .await
                    .map_err(theme_storage_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)
}

/// Replaces a fixed logo or header binding.
#[macros::server(skip_all)]
pub async fn replace_binding(
    scope: OwnershipScope,
    theme_id: ThemeId,
    role: ThemeImageRole,
    input: ThemeBindingInput,
) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    ensure_owned(owner, theme_id).await?;
    let manager = expect_context::<Arc<ThemeManager>>();
    manager
        .replace_role(
            actor,
            owner,
            theme_id,
            role,
            // The concrete manager tests exercise every storage role input. This is the
            // server-function serialization boundary, which has no host-only caller.
            // cov:ignore-start
            match input {
                ThemeBindingInput::PackagedDefault => ThemeRoleInput::PackagedDefault,
                ThemeBindingInput::ExplicitAbsent => ThemeRoleInput::ExplicitAbsent,
                ThemeBindingInput::PackageAsset(path) => ThemeRoleInput::PackageAsset(path),
                ThemeBindingInput::Media(media) => ThemeRoleInput::Media(MediaRef {
                    source: media.source,
                    sha256: media.sha256,
                    filename: media.filename,
                }),
            }, // cov:ignore-stop
        )
        .await
        .map_err(manager_error)
}

/// Replaces the full explicit header pool.
#[macros::server(skip_all)]
pub async fn replace_pool(
    scope: OwnershipScope,
    theme_id: ThemeId,
    inputs: Vec<ThemePoolInput>,
    shuffle_seed: [u8; 32],
) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    ensure_owned(owner, theme_id).await?;
    let manager = expect_context::<Arc<ThemeManager>>();
    manager
        .replace_header_pool(
            actor,
            owner,
            theme_id,
            inputs
                .into_iter()
                .map(|input| match input {
                    ThemePoolInput::PackageAsset(path) => {
                        storage::ThemePoolInput::PackageAsset(path)
                    }
                    ThemePoolInput::Media(media) => storage::ThemePoolInput::Media(MediaRef {
                        source: media.source,
                        sha256: media.sha256,
                        filename: media.filename,
                    }),
                })
                .collect(),
            shuffle_seed,
        )
        .await
        .map_err(manager_error)
}

/// Replaces only an existing header pool's deterministic shuffle assignment.
#[macros::server(skip_all)]
pub async fn shuffle(
    scope: OwnershipScope,
    theme_id: ThemeId,
    seed: [u8; 32],
) -> WebResult<MutationOutcome<()>> {
    let (actor, owner) = owner(scope).await?;
    ensure_owned(owner, theme_id).await?;
    let manager = expect_context::<Arc<ThemeManager>>();
    manager
        .shuffle_header_pool(actor, owner, theme_id, seed)
        .await
        .map_err(manager_error)
}

/// Renders an owned draft through the real Style Contract renderer without changing selection.
#[macros::server(skip_all)]
pub async fn preview(scope: OwnershipScope, theme_id: ThemeId) -> WebResult<ThemePreview> {
    let (actor, owner) = owner(scope).await?;
    no_store();
    let coordinator = expect_context::<Arc<ThemeOperationCoordinator>>();
    let _permit = coordinator.acquire(actor).map_err(admission_error)?;
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let draft = themes
        .get_draft(owner, theme_id)
        .await
        .map_err(InternalError::storage)?
        .ok_or_else(|| InternalError::not_found("theme"))?;
    let assets = draft
        .assets
        .iter()
        .map(|asset| (asset.path.clone(), asset.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let archive = theme_package::export_theme_package(&draft.manifest, &draft.stylesheet, &assets)
        .map_err(InternalError::server)?;
    let package = theme_package::validate_theme_package(&archive, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let urls = draft_asset_urls(&package, theme_id);
    let compiled = package
        .compile(&urls, ThemePackageLimits::default())
        .map_err(|error| InternalError::validation_source("invalid theme package", error))?;
    let preview_revision = draft
        .source_digest
        .to_string()
        .parse()
        .map_err(|_| InternalError::server_message("preview revision"))?;
    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let (page, route) = match owner {
        ThemeOwner::Site => (
            common::seed::PageSeed::SiteTimeline(
                crate::timeline::fetch_local_timeline(
                    expect_context::<Arc<dyn PostStorage>>().as_ref(),
                    &viewer,
                    None,
                    None,
                )
                .await?,
            ),
            common::theme::PublicThemeRoute::site(),
        ),
        ThemeOwner::Author(user_id) => {
            let user = expect_context::<Arc<dyn UserStorage>>()
                .get_user(user_id)
                .await
                .map_err(InternalError::storage)?
                .ok_or_else(|| InternalError::not_found("user"))?;
            (
                common::seed::PageSeed::Profile {
                    username: user.username.clone(),
                    page: crate::timeline::fetch_user_posts(
                        expect_context::<Arc<dyn PostStorage>>().as_ref(),
                        &viewer,
                        &user.username,
                        None,
                        None,
                    )
                    .await?,
                },
                common::theme::PublicThemeRoute::author(&user.username),
            )
        }
    };
    let (logo_url, header_url) = storage::resolve_draft_theme_images(
        owner,
        theme_id,
        &draft.manifest,
        &urls,
        &preview_revision,
        &route,
        themes.as_ref(),
    )
    .await
    .map_err(InternalError::storage)?;
    let presentation = common::seed::PublicPresentation {
        theme: common::theme::PublishedThemePresentation {
            identity: common::theme::PublishedThemeIdentity::Custom(theme_id),
            revision: Some(preview_revision),
            stylesheet_url: format!("/theme/{}", "0".repeat(64))
                .parse()
                .map_err(|_| InternalError::server_message("preview stylesheet URL"))?,
            logo_url,
            header_url,
        },
        page,
    };
    Ok(ThemePreview {
        html: crate::app::render_shell(&presentation).into_string(),
        css: String::from_utf8(compiled.css().bytes().to_vec())
            .map_err(|_| InternalError::server_message("compiler emitted non-UTF-8 CSS"))?,
    })
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{
        CatalogEntry, Draft, OwnershipScope, ThemeBindingInput, ThemeMediaInput, ThemePoolInput,
        admission_error, binding_wire, catalog, create_storage_error, digest_hex, draft_asset_urls,
        draft_from_archive, draft_from_input, draft_wire, get_selection, import_css,
        import_package, manager_error, media_wire, percent_encode_draft_asset_path,
        plain_css_manifest, pool_wire, publication_error, publish, remove, rename, replace_binding,
        replace_css, replace_draft_error, replace_pool, safe_filename, select, shuffle, theme_name,
        theme_storage_error,
    };
    use crate::{
        error::{WebError, project},
        test_support::auth_parts,
    };
    use common::{
        ids::{ThemeId, UserId},
        media::{MediaRef, MediaSource},
        theme::{ThemeImageRole, ThemePoolRevisionDigest},
    };
    use host::{
        theme_operations::{ThemeOperationCoordinator, ThemeOperationRejected},
        theme_package,
    };
    use leptos::prelude::{Owner, provide_context};
    use std::{collections::BTreeMap, sync::Arc};
    use storage::{
        MediaContentLocks, MockMediaStorage, MockThemeStorage, ReplaceDraftError, ThemeAssetError,
        ThemeAssetManager, ThemeCatalogEntry, ThemeHeaderPoolEntry, ThemeManager, ThemeOwner,
        ThemeRoleBinding, ThemeStorage, test_support::mock_write_scope,
    };

    fn assert_validation(error: &host::error::InternalError, message: &str) {
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Validation {
                message: message.into()
            }
        );
    }

    #[test]
    fn plain_css_manifest_is_a_complete_empty_package_manifest() {
        let manifest = plain_css_manifest("Ocean").unwrap();
        let value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();

        assert_eq!(value["name"], "Ocean");
        assert_eq!(value["schema"], 1);
        assert_eq!(value["assets"], serde_json::json!({}));
        assert_eq!(value["defaults"], serde_json::json!({}));
    }

    #[test]
    fn digest_and_draft_asset_urls_preserve_byte_identity_and_escape_paths() {
        assert_eq!(digest_hex(&[0, 15, 16, 255]), "000f10ff");
        assert_eq!(
            percent_encode_draft_asset_path("fonts/A B?#.woff2"),
            "fonts/A%20B%3F%23.woff2"
        );
        assert_eq!(
            percent_encode_draft_asset_path("safe-._~/path"),
            "safe-._~/path"
        );
    }

    #[test]
    fn draft_input_rejects_duplicate_paths_before_package_processing() {
        let error = draft_from_input(
            ThemeId::from(1_i64),
            Draft {
                manifest: Vec::new(),
                stylesheet: Vec::new(),
                assets: vec![
                    super::ThemeAssetInput {
                        path: "logo.png".into(),
                        mime: "image/png".into(),
                        bytes: vec![1],
                    },
                    super::ThemeAssetInput {
                        path: "logo.png".into(),
                        mime: "image/png".into(),
                        bytes: vec![2],
                    },
                ],
            },
        )
        .unwrap_err();

        assert_validation(&error, "duplicate theme asset path: logo.png");
    }

    #[test]
    fn archive_and_wire_round_trip_retains_complete_draft_source() {
        let manifest = plain_css_manifest("Ocean").unwrap();
        let archive = theme_package::export_theme_package(
            &manifest,
            b".j-theme-root { color: blue; }",
            &BTreeMap::new(),
        )
        .unwrap();
        let draft = draft_from_archive(ThemeId::from(12_i64), &archive).unwrap();
        let wire = draft_wire(draft.clone());

        assert_eq!(wire.manifest, draft.manifest);
        assert_eq!(wire.stylesheet, draft.stylesheet);
        assert!(wire.assets.is_empty());
        assert!(draft_from_input(ThemeId::from(12_i64), wire).is_ok());
    }

    #[test]
    fn package_asset_drafts_retain_validated_bytes_and_expose_draft_urls() {
        let theme_id = ThemeId::from(12_i64);
        let manifest = br#"{"schema":1,"name":"Ocean","style_contract":1,"assets":{"assets/logo.png":"image/png"},"defaults":{}}"#.to_vec();
        let png = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\x00\x00\x00\x0b\x49\x44\x41\x54\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".to_vec();
        let input = Draft {
            manifest: manifest.clone(),
            stylesheet: b".j-theme-root { color: blue; }".to_vec(),
            assets: vec![super::ThemeAssetInput {
                path: "assets/logo.png".into(),
                mime: "image/png".into(),
                bytes: png.clone(),
            }],
        };
        let draft = draft_from_input(theme_id, input.clone()).unwrap();
        let archive = theme_package::export_theme_package(
            &manifest,
            &input.stylesheet,
            &BTreeMap::from([("assets/logo.png".into(), png)]),
        )
        .unwrap();
        let from_archive = draft_from_archive(theme_id, &archive).unwrap();
        let package = theme_package::validate_theme_package(
            &archive,
            theme_package::ThemePackageLimits::default(),
        )
        .unwrap();

        assert_eq!(draft.assets, from_archive.assets);
        assert_eq!(draft_wire(draft).assets, input.assets);
        assert_eq!(
            draft_asset_urls(&package, theme_id)["assets/logo.png"],
            "/theme/draft/12/assets/logo.png"
        );
    }

    #[test]
    fn catalog_and_filename_wires_hide_storage_details_and_produce_safe_download_names() {
        let entry = catalog(ThemeCatalogEntry {
            id: ThemeId::from(3_i64),
            owner: ThemeOwner::Site,
            name: "Ocean".into(),
            current_revision: None,
        });

        assert_eq!(
            entry,
            CatalogEntry {
                id: ThemeId::from(3_i64),
                name: "Ocean".into(),
                published: false,
            }
        );
        assert_eq!(
            safe_filename("Summer theme (final)!"),
            "Summerthemefinal.zip"
        );
        assert_eq!(safe_filename("🌊"), "theme.zip");
    }

    #[test]
    fn operation_and_storage_errors_preserve_actionable_public_classes() {
        let rate_limited = admission_error(ThemeOperationRejected::RateLimited);
        assert_validation(&rate_limited, "theme operation rate limited");
        let in_flight = admission_error(ThemeOperationRejected::InFlight);
        assert_validation(&in_flight, "theme operation already in progress");
        let quota = create_storage_error(sqlx::Error::RowNotFound);
        assert_validation(&quota, "theme quota exceeded");
        let not_found = theme_storage_error(sqlx::Error::RowNotFound);
        assert_eq!(
            project(not_found.kind(), not_found.public_message()),
            WebError::NotFound {
                message: "theme not found".into()
            }
        );
        let draft_quota = replace_draft_error(ReplaceDraftError::QuotaExceeded);
        assert_validation(&draft_quota, "theme draft quota exceeded");
        let publication = publication_error(ThemeAssetError::Storage(sqlx::Error::RowNotFound));
        assert_validation(&publication, "theme publication rejected");
    }

    #[test]
    fn internal_manager_and_draft_errors_keep_not_found_and_server_boundaries_distinct() {
        let missing_draft = replace_draft_error(ReplaceDraftError::OwnerNotFound);
        assert_eq!(
            project(missing_draft.kind(), missing_draft.public_message()),
            WebError::NotFound {
                message: "theme not found".into()
            }
        );
        let missing_manager = manager_error(anyhow::Error::from(sqlx::Error::RowNotFound));
        assert_eq!(
            project(missing_manager.kind(), missing_manager.public_message()),
            WebError::NotFound {
                message: "theme not found".into()
            }
        );
        let internal_manager = manager_error(anyhow::anyhow!("manager unavailable"));
        assert!(matches!(
            project(internal_manager.kind(), internal_manager.public_message()),
            WebError::Server { .. }
        ));
        let failed_publication = publication_error(ThemeAssetError::InvalidDigest);
        assert!(matches!(
            project(
                failed_publication.kind(),
                failed_publication.public_message()
            ),
            WebError::Server { .. }
        ));
    }

    #[test]
    fn binding_and_pool_wires_preserve_explicit_sources() {
        let theme_id = ThemeId::from(4_i64);
        assert_eq!(
            binding_wire(ThemeRoleBinding::PackagedDefault {
                theme_id,
                role: ThemeImageRole::Logo,
            })
            .unwrap(),
            ThemeBindingInput::PackagedDefault
        );
        assert_eq!(
            binding_wire(ThemeRoleBinding::ExplicitAbsent {
                theme_id,
                role: ThemeImageRole::Logo,
            })
            .unwrap(),
            ThemeBindingInput::ExplicitAbsent
        );
        assert_eq!(
            binding_wire(ThemeRoleBinding::PackageAsset {
                theme_id,
                role: ThemeImageRole::Logo,
                package_path: "logo.png".into(),
            })
            .unwrap(),
            ThemeBindingInput::PackageAsset("logo.png".into())
        );
        assert_eq!(
            pool_wire(ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: Some("header.png".into()),
                media_user_id: None,
                media_source: None,
                media_digest: None,
                media_filename: None,
            })
            .unwrap(),
            ThemePoolInput::PackageAsset("header.png".into())
        );
    }

    #[test]
    fn media_and_header_pool_wires_distinguish_complete_and_impossible_bindings() {
        let theme_id = ThemeId::from(4_i64);
        let media = MediaRef {
            source: MediaSource::Upload,
            sha256: "a".repeat(64).parse().unwrap(),
            filename: "hero.png".parse().unwrap(),
        };
        let expected = ThemeMediaInput {
            source: media.source,
            sha256: media.sha256.clone(),
            filename: media.filename.clone(),
        };
        assert_eq!(
            binding_wire(ThemeRoleBinding::Media {
                theme_id,
                role: ThemeImageRole::Header,
                user_id: UserId::from(7_i64),
                media,
            })
            .unwrap(),
            ThemeBindingInput::Media(expected.clone())
        );
        let error = binding_wire(ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision: "b".repeat(64).parse::<ThemePoolRevisionDigest>().unwrap(),
            shuffle_seed: [9; 32],
        })
        .unwrap_err();
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".into()
            }
        );
        assert_eq!(
            media_wire(
                Some("upload".into()),
                Some("a".repeat(64).parse().unwrap()),
                Some("hero.png".into()),
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn malformed_media_pool_is_rejected_at_the_first_missing_wire_field() {
        let error = pool_wire(ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        })
        .unwrap_err();

        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".into()
            }
        );
    }

    #[test]
    fn theme_name_uses_catalog_validation() {
        assert_eq!(theme_name("Ocean").unwrap(), "Ocean");
        assert!(theme_name("").is_err());
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn selection_reads_only_the_authenticated_authors_catalog() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        let mut themes = MockThemeStorage::new();
        themes
            .expect_selection()
            .withf(|owner| matches!(owner, ThemeOwner::Author(id) if *id == UserId::from(7_i64)))
            .returning(|_| Ok(None));
        provide_context(Arc::new(themes) as Arc<dyn ThemeStorage>);

        let selection = get_selection(OwnershipScope::Author).await.unwrap();

        drop(reactive_owner);
        assert_eq!(selection, None);
    }
    // guard:no-backend — mock store and write scope
    #[tokio::test]
    async fn importing_a_valid_owned_draft_replaces_it_transactionally() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let input = Draft {
            manifest: plain_css_manifest("Ocean").unwrap(),
            stylesheet: b".j-theme-root { color: blue; }".to_vec(),
            assets: Vec::new(),
        };
        let existing = draft_from_input(theme_id, input.clone()).unwrap();

        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, id| Ok((id == theme_id).then(|| existing.clone())));
        themes.expect_replace_draft().returning(|_, _, _, _| Ok(()));
        provide_author_and_themes(themes);
        provide_context(mock_write_scope());

        let outcome = import_package(OwnershipScope::Author, theme_id, input)
            .await
            .unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }
    // guard:no-backend — mock store and write scope
    #[tokio::test]
    async fn importing_css_creates_a_valid_private_draft_and_rejects_oversized_source() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_create_theme()
            .withf(|_, owner, name, draft, _| {
                *owner == ThemeOwner::Author(UserId::from(7_i64))
                    && name == "Ocean"
                    && draft.manifest == plain_css_manifest("Ocean").unwrap()
                    && draft.stylesheet == b".j-theme-root { color: blue; }"
                    && draft.assets.is_empty()
            })
            .returning(|_, _, _, _, _| Ok(ThemeId::from(9_i64)));
        provide_author_and_themes(themes);
        provide_context(mock_write_scope());

        let outcome = import_css(
            OwnershipScope::Author,
            "Ocean".into(),
            b".j-theme-root { color: blue; }".to_vec(),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            common::MutationOutcome::Confirmed(CatalogEntry {
                id: ThemeId::from(9_i64),
                name: "Ocean".into(),
                published: false,
            })
        );
        let error = import_css(
            OwnershipScope::Author,
            "Ocean".into(),
            vec![0; theme_package::ThemePackageLimits::default().max_file_bytes + 1],
        )
        .await
        .unwrap_err();
        drop(reactive_owner);
        assert_eq!(
            error,
            WebError::Validation {
                message: "theme stylesheet is too large".into()
            }
        );
    }

    // guard:no-backend — mock store and write scope
    #[tokio::test]
    async fn rename_and_clear_selection_commit_author_owned_changes() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let mut themes = MockThemeStorage::new();
        themes.expect_rename_theme().returning(|_, _, _, _| Ok(()));
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::new(themes) as Arc<dyn ThemeStorage>);
        provide_context(mock_write_scope());

        assert_eq!(
            rename(OwnershipScope::Author, ThemeId::from(8_i64), "Ocean".into())
                .await
                .unwrap(),
            common::MutationOutcome::Confirmed(())
        );
        drop(reactive_owner);

        let reactive_owner = Owner::new();
        reactive_owner.set();
        let mut themes = MockThemeStorage::new();
        themes.expect_set_selection().returning(|_, _, _| Ok(()));
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::new(themes) as Arc<dyn ThemeStorage>);
        provide_context(mock_write_scope());

        assert_eq!(
            select(OwnershipScope::Author, None).await.unwrap(),
            common::MutationOutcome::Confirmed(())
        );
        drop(reactive_owner);
    }

    // guard:no-backend — concrete manager over mock stores and write scope
    #[tokio::test]
    async fn replacing_a_packaged_default_binding_commits_owned_mutation() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let existing = draft_from_input(
            theme_id,
            Draft {
                manifest: plain_css_manifest("Ocean").unwrap(),
                stylesheet: b".j-theme-root { color: blue; }".to_vec(),
                assets: Vec::new(),
            },
        )
        .unwrap();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, _| Ok(Some(existing.clone())));
        themes.expect_role_binding().returning(|_, _, _| Ok(None));
        themes.expect_header_pool().returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_replace_role_binding()
            .returning(|_, _, _| Ok(()));
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let manager = ThemeManager::new(
            Arc::clone(&themes),
            Arc::new(MockMediaStorage::new()),
            mock_write_scope(),
            Arc::new(MediaContentLocks::new(Arc::new(std::env::temp_dir()))),
        );
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::clone(&themes));
        provide_context(Arc::new(manager));

        let outcome = replace_binding(
            OwnershipScope::Author,
            theme_id,
            ThemeImageRole::Logo,
            ThemeBindingInput::PackagedDefault,
        )
        .await
        .unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }

    // guard:no-backend — concrete manager over mock stores and write scope
    #[tokio::test]
    async fn replacing_an_owned_header_pool_preserves_wire_inputs() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let draft = draft_from_input(
            theme_id,
            Draft {
                manifest: plain_css_manifest("Ocean").unwrap(),
                stylesheet: b".j-theme-root { color: blue; }".to_vec(),
                assets: Vec::new(),
            },
        )
        .unwrap();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, _| Ok(Some(draft.clone())));
        themes
            .expect_role_binding()
            .times(2)
            .returning(|_, _, _| Ok(None));
        themes.expect_header_pool().returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_replace_header_pool()
            .withf(|_, owner, id, entries| {
                *owner == ThemeOwner::Author(UserId::from(7_i64))
                    && *id == ThemeId::from(8_i64)
                    && entries.iter().any(|entry| {
                        entry.package_path.as_deref() == Some("headers/default.png")
                            && entry.media_user_id.is_none()
                    })
                    && entries.iter().any(|entry| {
                        entry.package_path.is_none()
                            && entry.media_user_id == Some(UserId::from(7_i64))
                            && entry.media_source.as_deref() == Some("upload")
                            && entry.media_filename.as_deref() == Some("hero.png")
                    })
            })
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_replace_role_binding()
            .returning(|_, _, binding| {
                assert!(matches!(
                    binding,
                    ThemeRoleBinding::HeaderPool { shuffle_seed, .. } if *shuffle_seed == [3; 32]
                ));
                Ok(())
            });
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let mut media_storage = MockMediaStorage::new();
        media_storage
            .expect_lock_media_reference()
            .withf(|_, media| {
                media.source == MediaSource::Upload && media.filename.as_ref() == "hero.png"
            })
            .returning(|_, _| Ok(()));
        let manager = ThemeManager::new(
            Arc::clone(&themes),
            Arc::new(media_storage),
            mock_write_scope(),
            Arc::new(MediaContentLocks::new(Arc::new(std::env::temp_dir()))),
        );
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::clone(&themes));
        provide_context(Arc::new(manager));

        let outcome = replace_pool(
            OwnershipScope::Author,
            theme_id,
            vec![
                ThemePoolInput::PackageAsset("headers/default.png".into()),
                ThemePoolInput::Media(ThemeMediaInput {
                    source: MediaSource::Upload,
                    sha256: "a".repeat(64).parse().unwrap(),
                    filename: "hero.png".parse().unwrap(),
                }),
            ],
            [3; 32],
        )
        .await
        .unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }

    // guard:no-backend — concrete manager over mock stores and write scope
    #[tokio::test]
    async fn shuffling_an_owned_header_pool_preserves_the_seed() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let mut themes = MockThemeStorage::new();
        themes.expect_get_draft().returning(|_, _| {
            Ok(Some(
                draft_from_input(
                    ThemeId::from(8_i64),
                    Draft {
                        manifest: plain_css_manifest("Ocean").unwrap(),
                        stylesheet: b".j-theme-root { color: blue; }".to_vec(),
                        assets: Vec::new(),
                    },
                )
                .unwrap(),
            ))
        });
        themes.expect_role_binding().returning(move |_, _, _| {
            Ok(Some(ThemeRoleBinding::HeaderPool {
                theme_id,
                pool_revision: "b".repeat(64).parse::<ThemePoolRevisionDigest>().unwrap(),
                shuffle_seed: [4; 32],
            }))
        });
        themes
            .expect_shuffle_header_pool()
            .withf(|_, owner, id, seed| {
                *owner == ThemeOwner::Author(UserId::from(7_i64))
                    && *id == ThemeId::from(8_i64)
                    && *seed == [4; 32]
            })
            .returning(|_, _, _, _| Ok(()));
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let manager = ThemeManager::new(
            Arc::clone(&themes),
            Arc::new(MockMediaStorage::new()),
            mock_write_scope(),
            Arc::new(MediaContentLocks::new(Arc::new(std::env::temp_dir()))),
        );
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::clone(&themes));
        provide_context(Arc::new(manager));

        let outcome = shuffle(OwnershipScope::Author, theme_id, [4; 32])
            .await
            .unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }

    // guard:no-backend — concrete manager over mock stores and write scope
    #[tokio::test]
    async fn removing_an_owned_theme_commits_the_manager_mutation() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let draft = draft_from_input(
            theme_id,
            Draft {
                manifest: plain_css_manifest("Ocean").unwrap(),
                stylesheet: b".j-theme-root { color: blue; }".to_vec(),
                assets: Vec::new(),
            },
        )
        .unwrap();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, _| Ok(Some(draft.clone())));
        themes
            .expect_role_binding()
            .times(2)
            .returning(|_, _, _| Ok(None));
        themes.expect_header_pool().returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_remove_theme()
            .withf(|_, owner, id, _| {
                *owner == ThemeOwner::Author(UserId::from(7_i64)) && *id == ThemeId::from(8_i64)
            })
            .returning(|_, _, _, _| Ok(()));
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let manager = ThemeManager::new(
            Arc::clone(&themes),
            Arc::new(MockMediaStorage::new()),
            mock_write_scope(),
            Arc::new(MediaContentLocks::new(Arc::new(std::env::temp_dir()))),
        );
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::clone(&themes));
        provide_context(Arc::new(manager));

        let outcome = remove(OwnershipScope::Author, theme_id).await.unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }
    // guard:no-backend — concrete asset manager over a per-test content root
    #[tokio::test]
    async fn publishing_a_valid_owned_draft_records_an_immutable_revision() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let draft = draft_from_input(
            theme_id,
            Draft {
                manifest: plain_css_manifest("Ocean").unwrap(),
                stylesheet: b".j-theme-root { color: blue; }".to_vec(),
                assets: Vec::new(),
            },
        )
        .unwrap();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, _| Ok(Some(draft.clone())));
        themes.expect_admit_publication().returning(|_, _| Ok(()));
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let content_root = Arc::new(
            std::env::temp_dir().join(format!("jaunder-theme-publish-{}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(content_root.as_ref());
        let manager = ThemeAssetManager::new(
            Arc::clone(&themes),
            mock_write_scope(),
            Arc::clone(&content_root),
        );
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::new(ThemeOperationCoordinator::new()));
        provide_context(Arc::clone(&themes));
        provide_context(Arc::new(manager));

        let outcome = publish(OwnershipScope::Author, theme_id).await.unwrap();

        drop(reactive_owner);
        let _ = std::fs::remove_dir_all(content_root.as_ref());
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));
    }

    // guard:no-backend — mock store and write scope
    #[tokio::test]
    async fn replacing_css_retains_owned_package_fields_and_commits() {
        let reactive_owner = Owner::new();
        reactive_owner.set();
        let theme_id = ThemeId::from(8_i64);
        let original = Draft {
            manifest: plain_css_manifest("Ocean").unwrap(),
            stylesheet: b".j-theme-root { color: blue; }".to_vec(),
            assets: Vec::new(),
        };
        let existing = draft_from_input(theme_id, original.clone()).unwrap();
        let expected_manifest = original.manifest.clone();
        let mut themes = MockThemeStorage::new();
        themes
            .expect_get_draft()
            .returning(move |_, _| Ok(Some(existing.clone())));
        themes
            .expect_replace_draft()
            .returning(move |_, _, draft, _| {
                assert_eq!(draft.manifest, expected_manifest);
                assert_eq!(draft.stylesheet, b".j-theme-root { color: red; }");
                Ok(())
            });
        provide_author_and_themes(themes);
        provide_context(mock_write_scope());

        let outcome = replace_css(
            OwnershipScope::Author,
            theme_id,
            b".j-theme-root { color: red; }".to_vec(),
        )
        .await
        .unwrap();

        drop(reactive_owner);
        assert_eq!(outcome, common::MutationOutcome::Confirmed(()));
    }

    fn provide_author_and_themes(themes: MockThemeStorage) {
        provide_context(auth_parts(UserId::from(7_i64), "author"));
        provide_context(Arc::new(ThemeOperationCoordinator::new()));
        provide_context(Arc::new(themes) as Arc<dyn ThemeStorage>);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn draft_mutations_mask_missing_owned_theme_before_processing_input() {
        #[derive(Clone, Copy, Debug)]
        enum Operation {
            ImportPackage,
            ReplaceStylesheet,
            Publish,
            ReplaceBinding,
        }

        for operation in [
            Operation::ImportPackage,
            Operation::ReplaceStylesheet,
            Operation::Publish,
            Operation::ReplaceBinding,
        ] {
            let reactive_owner = Owner::new();
            reactive_owner.set();
            let mut themes = MockThemeStorage::new();
            themes.expect_get_draft().returning(|_, _| Ok(None));
            provide_author_and_themes(themes);

            let result = match operation {
                Operation::ImportPackage => {
                    import_package(
                        OwnershipScope::Author,
                        ThemeId::from(8_i64),
                        Draft {
                            manifest: Vec::new(),
                            stylesheet: Vec::new(),
                            assets: Vec::new(),
                        },
                    )
                    .await
                }
                Operation::ReplaceStylesheet => {
                    replace_css(OwnershipScope::Author, ThemeId::from(8_i64), Vec::new()).await
                }
                Operation::Publish => publish(OwnershipScope::Author, ThemeId::from(8_i64)).await,
                Operation::ReplaceBinding => {
                    replace_binding(
                        OwnershipScope::Author,
                        ThemeId::from(8_i64),
                        ThemeImageRole::Logo,
                        ThemeBindingInput::ExplicitAbsent,
                    )
                    .await
                }
            };
            drop(reactive_owner);
            assert!(
                matches!(result, Err(WebError::NotFound { .. })),
                "{operation:?}"
            );
        }
    }
}
