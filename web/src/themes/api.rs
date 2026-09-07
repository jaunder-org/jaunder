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
        error::{InternalError, from_write_scope_error},
    },
    common::{media::MediaRef, theme},
    host::{
        theme_operations::{ThemeOperationCoordinator, ThemeOperationRejected},
        theme_package::{self, ThemePackageLimits, ValidatedThemePackage},
    },
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
fn multipart_error(error: multer::Error) -> InternalError {
    InternalError::validation_source("invalid multipart theme import", error)
}

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
    Ok(ThemeDraft {
        theme_id,
        manifest: package.canonical_manifest().to_vec(),
        stylesheet: package.authored_css().to_vec(),
        source_digest: digest_hex(&package.source_digest())
            .parse()
            .map_err(|_| InternalError::validation("invalid package digest"))?,
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
    Ok(ThemeDraft {
        theme_id,
        manifest: package.canonical_manifest().to_vec(),
        stylesheet: package.authored_css().to_vec(),
        source_digest: digest_hex(&package.source_digest())
            .parse()
            .map_err(|_| InternalError::server_message("validated source digest"))?,
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
            Ok((path.to_owned(), format!("/themes/{}", digest_hex(&digest))))
        })
        .collect()
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
                    "/themes/draft/{theme_id}/{}",
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
    let shuffle_seed = match &header_binding {
        Some(storage::ThemeRoleBinding::HeaderPool { shuffle_seed, .. }) => Some(*shuffle_seed),
        _ => None,
    };
    let header = match header_binding {
        Some(storage::ThemeRoleBinding::HeaderPool { .. }) => None,
        binding => binding.map(binding_wire).transpose()?,
    };
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
        .map_err(from_write_scope_error)?;
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
        .map_err(from_write_scope_error)?;
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
    let name = multipart
        .next_field()
        .await
        .map_err(multipart_error)?
        .ok_or_else(|| InternalError::validation("missing theme name"))?;
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
    let limit = ThemePackageLimits::default().max_archive_bytes;
    let mut bytes = Vec::new();
    while let Some(chunk) = archive.chunk().await.map_err(multipart_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(InternalError::validation("theme archive is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    drop(archive);
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
        .map_err(from_write_scope_error)?;
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
    )?;
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
        .map_err(from_write_scope_error)?;
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
        .map_err(from_write_scope_error)
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
    if let Some(options) = use_context::<ResponseOptions>() {
        options.insert_header(
            axum::http::header::CONTENT_DISPOSITION,
            axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .map_err(InternalError::server)?,
        );
    }
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
        .map_err(from_write_scope_error)
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
            chrono::Utc::now().timestamp() + storage::THEME_CONTENT_RETENTION_SECONDS,
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
            chrono::Utc::now().timestamp(),
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
        if !themes
            .list_themes(owner)
            .await
            .map_err(InternalError::storage)?
            .into_iter()
            .any(|entry| entry.id == theme_id && entry.current_revision.is_some())
        {
            return Err(InternalError::not_found("theme"));
        }
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
        .map_err(from_write_scope_error)
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
            match input {
                ThemeBindingInput::PackagedDefault => ThemeRoleInput::PackagedDefault,
                ThemeBindingInput::ExplicitAbsent => ThemeRoleInput::ExplicitAbsent,
                ThemeBindingInput::PackageAsset(path) => ThemeRoleInput::PackageAsset(path),
                ThemeBindingInput::Media(media) => ThemeRoleInput::Media(MediaRef {
                    source: media.source,
                    sha256: media.sha256,
                    filename: media.filename,
                }),
            },
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
            stylesheet_url: format!("/themes/{}", "0".repeat(64))
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
