//! Transactional orchestration for mutable public-theme image bindings.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use common::{
    MutationOutcome,
    ids::{ThemeId, UserId},
    media::{ContentHash, Filename, MediaRef, MediaSource},
    theme::{ThemeContentDigest, ThemeHeaderPool, ThemeImageRole, ThemePoolEntry},
};

use crate::{
    MediaContentLocks, MediaStorage, ThemeHeaderPoolEntry, ThemeOwner, ThemeRoleBinding,
    ThemeStorage, WriteScope, WriteScopeError, WriteTransaction,
};

/// A caller-supplied image binding. Media ownership is always derived from the actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeRoleInput {
    PackagedDefault,
    ExplicitAbsent,
    PackageAsset(String),
    Media(MediaRef),
}

/// A caller-supplied member of an explicit header pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemePoolInput {
    PackageAsset(String),
    Media(MediaRef),
}

/// Validated normal editor fields and the saved context they were based on.
pub struct ThemeEditorChange {
    pub name: String,
    pub expected: Option<crate::ThemeCopySnapshot>,
    pub draft: crate::ThemeDraft,
    /// Only explicitly changed roles occur here; absent roles retain exact provenance.
    pub replacements: Vec<ThemeRoleBinding>,
}

impl ThemeEditorChange {
    async fn persist(
        self,
        themes: &dyn ThemeStorage,
        transaction: &mut WriteTransaction,
        owner: ThemeOwner,
        name: String,
        candidate: Option<&crate::theme_asset_manager::PreparedThemePublication>,
        limits: crate::ThemeQuotaLimits,
    ) -> Result<ThemeEditorSaved> {
        let (id, active, previous_revision) = if let Some(expected) = self.expected {
            let current = themes
                .copy_snapshot(transaction, owner, expected.entry.id)
                .await?;
            if current != expected {
                bail!("theme source changed while acquiring locks");
            }
            let active = themes.selection_for_update(transaction, owner).await?
                == Some(common::theme::PublicThemeSelection::Custom(
                    expected.entry.id,
                ));
            themes
                .replace_draft(transaction, owner, &self.draft, limits)
                .await?;
            themes
                .rename_theme(transaction, owner, expected.entry.id, &name)
                .await?;
            (expected.entry.id, active, expected.entry.current_revision)
        } else {
            let id = themes
                .create_theme(transaction, owner, &name, &self.draft, limits)
                .await?;
            (id, false, None)
        };
        for binding in self.replacements {
            themes
                .replace_role_binding(transaction, owner, &copied_binding(binding, id))
                .await?;
        }
        let mut applied = false;
        if active {
            let publication = candidate.context("missing prepared editor publication")?;
            themes
                .admit_publication(transaction, publication.admission(owner, limits))
                .await?;
            applied = previous_revision.as_ref() != Some(&publication.revision.digest);
        }
        Ok(ThemeEditorSaved { id, name, applied })
    }
}

/// A transaction-classified editor receipt, not a claim about an uncertain commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeEditorSaved {
    pub id: ThemeId,
    pub name: String,
    pub applied: bool,
}

/// Root-injected coordinator for theme image bindings and header pools.
pub struct ThemeManager {
    themes: Arc<dyn ThemeStorage>,
    media: Arc<dyn MediaStorage>,
    content_locks: Arc<MediaContentLocks>,
    write_scope: WriteScope,
}

impl ThemeManager {
    #[must_use]
    pub fn new(
        themes: Arc<dyn ThemeStorage>,
        media: Arc<dyn MediaStorage>,
        write_scope: WriteScope,
        content_locks: Arc<MediaContentLocks>,
    ) -> Self {
        Self {
            themes,
            media,
            content_locks,
            write_scope,
        }
    }

    /// Replaces one fixed image role. The actor, rather than a public input, supplies
    /// the persisted media owner.
    ///
    /// # Errors
    ///
    /// Returns an error for unauthorized actors, invalid or missing package/Media
    /// references, lock or transaction failures, or a concurrent binding change.
    pub async fn replace_role(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        theme_id: ThemeId,
        role: ThemeImageRole,
        input: ThemeRoleInput,
    ) -> Result<MutationOutcome<()>> {
        Self::require_author_actor(actor, owner)?;
        let expected = self.media_snapshot(owner, theme_id).await?;
        let mut media = expected.clone();
        if let ThemeRoleInput::Media(reference) = &input {
            media.push(reference.clone());
            media.sort();
            media.dedup();
        }
        let binding = Self::binding(actor, theme_id, role, input);
        let expected_binding = binding.clone();
        let _content_locks = self.content_locks.acquire(media.iter()).await?;
        let themes = Arc::clone(&self.themes);
        let themes_for_revalidation = Arc::clone(&self.themes);
        let media_storage = Arc::clone(&self.media);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    lock_media_refs(media_storage.as_ref(), transaction, &media).await?;
                    if snapshot_for_update(themes.as_ref(), transaction, owner, theme_id).await?
                        != expected
                    {
                        bail!("theme media references changed while acquiring locks");
                    }
                    themes
                        .replace_role_binding(transaction, owner, &binding)
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(scope_error)?;
        if matches!(outcome, MutationOutcome::CommitIndeterminate(()))
            && themes_for_revalidation
                .role_binding(owner, theme_id, role)
                .await?
                == Some(expected_binding)
        {
            return Ok(MutationOutcome::Confirmed(()));
        }
        Ok(outcome)
    }

    /// Replaces the header's complete explicit pool and its derived canonical digest.
    ///
    /// # Errors
    ///
    /// Returns an error for unauthorized actors, an empty or duplicate pool,
    /// invalid or missing package/Media references, lock or transaction failures,
    /// or a concurrent binding change.
    pub async fn replace_header_pool(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        theme_id: ThemeId,
        inputs: Vec<ThemePoolInput>,
        shuffle_seed: [u8; 32],
    ) -> Result<MutationOutcome<()>> {
        Self::require_author_actor(actor, owner)?;
        let (pool, entries, mut media) = Self::pool(actor, inputs)?;
        let expected = self.media_snapshot(owner, theme_id).await?;
        media.extend(expected.clone());
        media.sort();
        media.dedup();
        let binding = ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision: pool.revision().clone(),
            shuffle_seed,
        };
        let expected_binding = binding.clone();
        let expected_entries = entries.clone();
        let _content_locks = self.content_locks.acquire(media.iter()).await?;
        let themes = Arc::clone(&self.themes);
        let themes_for_revalidation = Arc::clone(&self.themes);
        let media_storage = Arc::clone(&self.media);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    lock_media_refs(media_storage.as_ref(), transaction, &media).await?;
                    if snapshot_for_update(themes.as_ref(), transaction, owner, theme_id).await?
                        != expected
                    {
                        bail!("theme media references changed while acquiring locks");
                    }
                    // The storage primitive persists the pool while this role update remains
                    // in the same transaction, so the revision and entries cannot diverge.
                    themes
                        .replace_header_pool(transaction, owner, theme_id, &entries)
                        .await?;
                    themes
                        .replace_role_binding(transaction, owner, &binding)
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(scope_error)?;
        if matches!(outcome, MutationOutcome::CommitIndeterminate(()))
            && themes_for_revalidation
                .role_binding(owner, theme_id, ThemeImageRole::Header)
                .await?
                == Some(expected_binding)
            && themes_for_revalidation.header_pool(owner, theme_id).await? == expected_entries
        {
            return Ok(MutationOutcome::Confirmed(()));
        }
        Ok(outcome)
    }

    /// Replaces only an existing header-pool seed.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is unauthorized, the theme has no header
    /// pool, or the transaction fails.
    pub async fn shuffle_header_pool(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        theme_id: ThemeId,
        shuffle_seed: [u8; 32],
    ) -> Result<MutationOutcome<()>> {
        Self::require_author_actor(actor, owner)?;
        let themes = Arc::clone(&self.themes);
        let themes_for_revalidation = Arc::clone(&self.themes);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .shuffle_header_pool(transaction, owner, theme_id, shuffle_seed)
                        .await?;
                    Ok::<(), anyhow::Error>(())
                })
            })
            .await
            .map_err(scope_error)?;
        if matches!(
            themes_for_revalidation
                .role_binding(owner, theme_id, ThemeImageRole::Header)
                .await?,
            Some(ThemeRoleBinding::HeaderPool {
                shuffle_seed: actual_seed,
                ..
            }) if actual_seed == shuffle_seed
        ) {
            return Ok(MutationOutcome::Confirmed(()));
        }
        Ok(outcome)
    }
    /// Removes a theme only while its complete media-reference snapshot remains
    /// stable. Theme bytes remain retained for `ThemeAssetManager` collection.
    ///
    /// # Errors
    ///
    /// Returns an error for unauthorized actors, lock or transaction failures,
    /// inconsistent retention accounting, or a concurrent binding change.
    pub async fn remove_theme(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        theme_id: ThemeId,
        retained_until_unix_seconds: i64,
    ) -> Result<MutationOutcome<()>> {
        Self::require_author_actor(actor, owner)?;
        let snapshot = self.media_snapshot(owner, theme_id).await?;
        let _content_locks = self.content_locks.acquire(snapshot.iter()).await?;
        let themes = Arc::clone(&self.themes);
        let themes_for_revalidation = Arc::clone(&self.themes);
        let media_storage = Arc::clone(&self.media);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    lock_media_refs(media_storage.as_ref(), transaction, &snapshot).await?;
                    let current =
                        snapshot_for_update(themes.as_ref(), transaction, owner, theme_id).await?;
                    if current != snapshot {
                        bail!("theme media references changed while acquiring locks");
                    }
                    themes
                        .remove_theme(transaction, owner, theme_id, retained_until_unix_seconds)
                        .await?;
                    Ok(())
                })
            })
            .await
            .map_err(scope_error)?;
        if matches!(outcome, MutationOutcome::CommitIndeterminate(()))
            && !themes_for_revalidation
                .list_themes(owner)
                .await?
                .iter()
                .any(|theme| theme.id == theme_id)
        {
            return Ok(MutationOutcome::Confirmed(()));
        }
        Ok(outcome)
    }

    /// Reads the candidate saved source before Media lock acquisition. The writer
    /// compares every field with a coherent transactional read before admission.
    ///
    /// # Errors
    /// Returns not-found for a foreign source and propagates storage failures.
    pub async fn copy_source(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
    ) -> Result<crate::ThemeCopySnapshot> {
        let entry = self
            .themes
            .list_themes(owner)
            .await?
            .into_iter()
            .find(|entry| entry.id == theme_id)
            .ok_or(sqlx::Error::RowNotFound)?;
        let draft = self
            .themes
            .get_draft(owner, theme_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        let mut bindings = Vec::new();
        for role in [ThemeImageRole::Logo, ThemeImageRole::Header] {
            if let Some(binding) = self.themes.role_binding(owner, theme_id, role).await? {
                bindings.push(binding);
            }
        }
        let header_pool = self.themes.header_pool(owner, theme_id).await?;
        Ok(crate::ThemeCopySnapshot {
            entry,
            draft,
            bindings,
            header_pool,
        })
    }

    /// Copies a validated saved package and all authorized presentation bindings
    /// into one new private catalog entry, without publication or selection.
    ///
    /// # Errors
    /// Rejects changed sources, unavailable Media, foreign Media provenance,
    /// invalid bindings, quotas, or transaction/lock failures as a whole.
    pub async fn copy_saved(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        expected: crate::ThemeCopySnapshot,
        draft: crate::ThemeDraft,
    ) -> Result<MutationOutcome<crate::ThemeCatalogEntry>> {
        Self::require_author_actor(actor, owner)?;
        let mut saved_draft = expected.draft.clone();
        saved_draft.theme_id = draft.theme_id;
        if saved_draft != draft {
            bail!("validated package does not match saved source");
        }
        let mut references = Vec::new();
        for binding in &expected.bindings {
            if let ThemeRoleBinding::Media { user_id, media, .. } = binding {
                if *user_id != actor {
                    bail!("copy cannot reuse another user's Media");
                }
                references.push(media.clone());
            }
        }
        for entry in &expected.header_pool {
            if let Some(user_id) = entry.media_user_id {
                if user_id != actor {
                    bail!("copy cannot reuse another user's Media");
                }
                references.push(reference_from_fields(
                    entry.media_source.clone(),
                    entry.media_digest.clone(),
                    entry.media_filename.clone(),
                )?);
            }
        }
        references.sort();
        references.dedup();
        let _locks = self.content_locks.acquire(references.iter()).await?;
        let themes = Arc::clone(&self.themes);
        let media = Arc::clone(&self.media);
        self.write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    lock_media_refs(media.as_ref(), transaction, &references).await?;
                    let current = themes
                        .copy_snapshot(transaction, owner, expected.entry.id)
                        .await?;
                    if current != expected {
                        bail!("theme source changed while acquiring locks");
                    }
                    let entry = themes
                        .create_theme_automatic(
                            transaction,
                            owner,
                            &expected.entry.name,
                            &draft,
                            crate::ThemeQuotaLimits::production(),
                        )
                        .await?;
                    if !expected.header_pool.is_empty() {
                        themes
                            .replace_header_pool(
                                transaction,
                                owner,
                                entry.id,
                                &expected.header_pool,
                            )
                            .await?;
                    }
                    for binding in expected.bindings {
                        let binding = copied_binding(binding, entry.id);
                        themes
                            .replace_role_binding(transaction, owner, &binding)
                            .await?;
                    }
                    Ok(entry)
                })
            })
            .await
            .map_err(scope_error)
    }

    /// Saves all normal fields in one transaction, admitting publication only when
    /// the same owner's actual selection still names this existing theme.
    ///
    /// # Errors
    /// Rejects stale package/presentation context, unauthorized Media, quotas,
    /// name collisions and filesystem/transaction failures without a partial save.
    pub async fn save_editor(
        &self,
        actor: UserId,
        owner: ThemeOwner,
        change: ThemeEditorChange,
        assets: &crate::ThemeAssetManager,
        compiled: &host::theme_package::CompiledThemeRevision,
        limits: crate::ThemeQuotaLimits,
    ) -> Result<MutationOutcome<ThemeEditorSaved>> {
        Self::require_author_actor(actor, owner)?;
        let name = crate::validate_theme_catalog_name(&change.name)?;
        let mut references = match &change.expected {
            Some(snapshot) => {
                if snapshot.draft.manifest != change.draft.manifest
                    || snapshot.draft.assets != change.draft.assets
                {
                    bail!("editor cannot replace saved package context");
                }
                snapshot_media(snapshot)?
            }
            None => Vec::new(),
        };
        for binding in &change.replacements {
            if let ThemeRoleBinding::Media { user_id, media, .. } = binding {
                if *user_id != actor {
                    bail!("replacement Media must belong to the actor");
                }
                references.push(media.clone());
            }
            if matches!(binding, ThemeRoleBinding::HeaderPool { .. }) {
                bail!("header pools are not normal editor input");
            }
        }
        references.sort();
        references.dedup();
        let _media_locks = self.content_locks.acquire(references.iter()).await?;
        // Stage before taking database locks: concurrent selection is classified only
        // after staging, under the selection writers' shared serialization boundary.
        let prepared = match &change.expected {
            Some(snapshot) => Some(Arc::new(
                assets
                    .prepare_publication(
                        snapshot.entry.id,
                        compiled,
                        jiff::Timestamp::now().as_second(),
                    )
                    .await?,
            )),
            None => None,
        };
        let candidate = prepared.clone();
        let themes = Arc::clone(&self.themes);
        let media = Arc::clone(&self.media);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    lock_media_refs(media.as_ref(), transaction, &references).await?;
                    change
                        .persist(
                            themes.as_ref(),
                            transaction,
                            owner,
                            name,
                            candidate.as_deref(),
                            limits,
                        )
                        .await
                })
            })
            .await
            .map_err(scope_error);
        if let Some(prepared) = &prepared {
            // A confirmed inactive save admitted no content; remove only newly
            // installed unreferenced files. Unknown commits retain all staged bytes.
            if outcome.is_err()
                || matches!(&outcome, Ok(MutationOutcome::Confirmed(saved)) if !saved.applied)
            {
                assets.discard_prepared(prepared).await;
            }
        }
        if let Ok(MutationOutcome::CommitIndeterminate(saved)) = &outcome {
            // Reconcile once for diagnostic truth, never promote uncertainty or replay.
            if let Err(error) = self.themes.get_draft(owner, saved.id).await {
                host::error::report_swallowed(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "storage.theme_editor.reconcile",
                    host::error::SwallowedSource::Error(&error),
                );
            }
        }
        outcome.map(|outcome| match outcome {
            MutationOutcome::CommitIndeterminate(mut saved) => {
                saved.applied = false;
                MutationOutcome::CommitIndeterminate(saved)
            }
            confirmed @ MutationOutcome::Confirmed(_) => confirmed,
        })
    }

    async fn media_snapshot(&self, owner: ThemeOwner, theme_id: ThemeId) -> Result<Vec<MediaRef>> {
        snapshot_for(self.themes.as_ref(), owner, theme_id).await
    }

    fn require_author_actor(actor: UserId, owner: ThemeOwner) -> Result<()> {
        if let ThemeOwner::Author(author) = owner
            && actor != author
        {
            bail!("actor does not own author theme catalog");
        }
        Ok(())
    }

    fn binding(
        actor: UserId,
        theme_id: ThemeId,
        role: ThemeImageRole,
        input: ThemeRoleInput,
    ) -> ThemeRoleBinding {
        match input {
            ThemeRoleInput::PackagedDefault => ThemeRoleBinding::PackagedDefault { theme_id, role },
            ThemeRoleInput::ExplicitAbsent => ThemeRoleBinding::ExplicitAbsent { theme_id, role },
            ThemeRoleInput::PackageAsset(package_path) => ThemeRoleBinding::PackageAsset {
                theme_id,
                role,
                package_path,
            },
            ThemeRoleInput::Media(media) => ThemeRoleBinding::Media {
                theme_id,
                role,
                user_id: actor,
                media,
            },
        }
    }

    fn pool(
        actor: UserId,
        inputs: Vec<ThemePoolInput>,
    ) -> Result<(ThemeHeaderPool, Vec<ThemeHeaderPoolEntry>, Vec<MediaRef>)> {
        let mut canonical = Vec::with_capacity(inputs.len());
        let mut entries = Vec::with_capacity(inputs.len());
        let mut media = Vec::new();
        for input in inputs {
            match input {
                ThemePoolInput::PackageAsset(path) => {
                    canonical.push(ThemePoolEntry::Package(path.clone()));
                    entries.push(ThemeHeaderPoolEntry {
                        ordinal: 0,
                        package_path: Some(path),
                        media_user_id: None,
                        media_source: None,
                        media_digest: None,
                        media_filename: None,
                    });
                }
                ThemePoolInput::Media(reference) => {
                    canonical.push(pool_entry_for_media(&reference)?);
                    entries.push(ThemeHeaderPoolEntry {
                        ordinal: 0,
                        package_path: None,
                        media_user_id: Some(actor),
                        media_source: Some(reference.source.to_string()),
                        media_digest: Some(
                            reference
                                .sha256
                                .to_string()
                                .parse()
                                .context("media digest is not a theme content digest")?,
                        ),
                        media_filename: Some(reference.filename.to_string()),
                    });
                    media.push(reference);
                }
            }
        }
        let pool = ThemeHeaderPool::new(canonical).map_err(|error| anyhow!(error))?;
        // Storage repeats canonicalization before writing; derive the persisted order
        // from the common canonical entries so the digest and ordinals agree.
        entries.sort_by_key(header_entry_encoding);
        for (ordinal, entry) in entries.iter_mut().enumerate() {
            entry.ordinal = i64::try_from(ordinal).context("theme pool has too many entries")?;
        }
        media.sort_by_key(media_sort_key);
        media.dedup_by(|left, right| media_sort_key(left) == media_sort_key(right));
        Ok((pool, entries, media))
    }
}

fn snapshot_media(snapshot: &crate::ThemeCopySnapshot) -> Result<Vec<MediaRef>> {
    let mut references = Vec::new();
    for binding in &snapshot.bindings {
        if let ThemeRoleBinding::Media { media, .. } = binding {
            references.push(media.clone());
        }
    }
    for entry in &snapshot.header_pool {
        if entry.media_user_id.is_some() {
            references.push(reference_from_fields(
                entry.media_source.clone(),
                entry.media_digest.clone(),
                entry.media_filename.clone(),
            )?);
        }
    }
    Ok(references)
}

fn copied_binding(binding: ThemeRoleBinding, theme_id: ThemeId) -> ThemeRoleBinding {
    match binding {
        ThemeRoleBinding::PackagedDefault { role, .. } => {
            ThemeRoleBinding::PackagedDefault { theme_id, role }
        }
        ThemeRoleBinding::ExplicitAbsent { role, .. } => {
            ThemeRoleBinding::ExplicitAbsent { theme_id, role }
        }
        ThemeRoleBinding::PackageAsset {
            role, package_path, ..
        } => ThemeRoleBinding::PackageAsset {
            theme_id,
            role,
            package_path,
        },
        ThemeRoleBinding::Media {
            role,
            user_id,
            media,
            ..
        } => ThemeRoleBinding::Media {
            theme_id,
            role,
            user_id,
            media,
        },
        ThemeRoleBinding::HeaderPool {
            pool_revision,
            shuffle_seed,
            ..
        } => ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision,
            shuffle_seed,
        },
    }
}

fn pool_entry_for_media(media: &MediaRef) -> Result<ThemePoolEntry> {
    Ok(ThemePoolEntry::Media {
        source: media.source.to_string(),
        digest: media
            .sha256
            .to_string()
            .parse::<ThemeContentDigest>()
            .context("media digest is not a theme content digest")?,
        filename: media.filename.to_string(),
    })
}

fn header_entry_encoding(entry: &ThemeHeaderPoolEntry) -> Vec<u8> {
    match (
        &entry.package_path,
        entry.media_user_id,
        &entry.media_source,
        &entry.media_digest,
        &entry.media_filename,
    ) {
        (Some(path), None, None, None, None) => {
            ThemePoolEntry::Package(path.clone()).canonical_encoding()
        }
        (None, Some(_), Some(source), Some(digest), Some(filename)) => ThemePoolEntry::Media {
            source: source.clone(),
            digest: digest.clone(),
            filename: filename.clone(),
        }
        .canonical_encoding(),
        _ => Vec::new(),
    }
}

fn media_sort_key(media: &MediaRef) -> (String, String, String) {
    (
        media.sha256.to_string(),
        media.source.to_string(),
        media.filename.to_string(),
    )
}
async fn snapshot_for(
    themes: &dyn ThemeStorage,
    owner: ThemeOwner,
    theme_id: ThemeId,
) -> Result<Vec<MediaRef>> {
    let mut references = Vec::new();
    for role in [ThemeImageRole::Logo, ThemeImageRole::Header] {
        if let Some(ThemeRoleBinding::Media { media, .. }) =
            themes.role_binding(owner, theme_id, role).await?
        {
            references.push(media);
        }
    }
    for entry in themes.header_pool(owner, theme_id).await? {
        if entry.media_user_id.is_some() {
            references.push(reference_from_fields(
                entry.media_source,
                entry.media_digest,
                entry.media_filename,
            )?);
        }
    }
    references.sort();
    references.dedup();
    Ok(references)
}

async fn snapshot_for_update(
    themes: &dyn ThemeStorage,
    transaction: &mut WriteTransaction,
    owner: ThemeOwner,
    theme_id: ThemeId,
) -> Result<Vec<MediaRef>> {
    let mut references = themes
        .locked_media_references(transaction, owner, theme_id)
        .await?
        .into_iter()
        .map(|reference| reference.media)
        .collect::<Vec<_>>();
    references.sort();
    references.dedup();
    Ok(references)
}

fn reference_from_fields(
    source: Option<String>,
    digest: Option<ThemeContentDigest>,
    filename: Option<String>,
) -> Result<MediaRef> {
    Ok(MediaRef {
        source: source
            .context("media binding is missing source")?
            .parse::<MediaSource>()
            .context("media binding has invalid source")?,
        sha256: digest
            .context("media binding is missing digest")?
            .to_string()
            .parse::<ContentHash>()
            .context("media binding has invalid digest")?,
        filename: filename
            .context("media binding is missing filename")?
            .parse::<Filename>()
            .context("media binding has invalid filename")?,
    })
}
fn scope_error(error: WriteScopeError<anyhow::Error>) -> anyhow::Error {
    match error {
        WriteScopeError::Operation(error) => error,
        WriteScopeError::Begin(error) => error.into(),
    }
}

async fn lock_media_refs(
    media: &dyn MediaStorage,
    transaction: &mut crate::WriteTransaction,
    references: &[MediaRef],
) -> Result<()> {
    for reference in references {
        media.lock_media_reference(transaction, reference).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::theme::ThemeImageRole;
    use rstest::*;
    use rstest_reuse::*;

    use super::*;
    use crate::{
        ThemeAssetManager, ThemeDraft, ThemeOwner, ThemePublicationAdmission, ThemeRevision,
        test_support::{
            Backend, SeedUser, backends, compiled_theme_fixture, confirmed, create_site_theme,
            seed_media, theme_quota_limits,
        },
    };
    fn editor_change(
        snapshot: crate::ThemeCopySnapshot,
    ) -> (
        ThemeEditorChange,
        host::theme_package::CompiledThemeRevision,
    ) {
        let archive = host::theme_package::export_theme_package(
            &snapshot.draft.manifest,
            b"body { color: blue; }",
            &snapshot
                .draft
                .assets
                .iter()
                .map(|asset| (asset.path.clone(), asset.bytes.clone()))
                .collect(),
        )
        .unwrap();
        let package = host::theme_package::validate_theme_package(
            &archive,
            host::theme_package::ThemePackageLimits::default(),
        )
        .unwrap();
        let mut draft = snapshot.draft.clone();
        draft.stylesheet = package.authored_css().to_vec();
        draft.source_digest =
            common::theme::ThemeSourceDigest::from_digest(package.source_digest());
        let urls = package
            .asset_paths()
            .map(|path| {
                let (_, _, digest) = package.asset(path).unwrap();
                (
                    path.to_owned(),
                    common::theme::ThemeAssetDigest::from_digest(digest)
                        .content_url()
                        .to_string(),
                )
            })
            .collect();
        let compiled = package
            .compile(&urls, host::theme_package::ThemePackageLimits::default())
            .unwrap();
        (
            ThemeEditorChange {
                name: "Editor saved".into(),
                expected: Some(snapshot),
                draft,
                replacements: Vec::new(),
            },
            compiled,
        )
    }

    fn copy_fixture() -> crate::ThemeCopySnapshot {
        let compiled = compiled_theme_fixture();
        crate::ThemeCopySnapshot {
            entry: crate::ThemeCatalogEntry {
                id: ThemeId::from(1),
                owner: ThemeOwner::Site,
                name: "Source".into(),
                current_revision: None,
            },
            draft: ThemeDraft {
                theme_id: ThemeId::from(1),
                manifest: compiled.canonical_manifest().to_vec(),
                stylesheet: b"body { color: black; }".to_vec(),
                source_digest: "a".repeat(64).parse().unwrap(),
                assets: compiled
                    .assets()
                    .map(|(path, mime, bytes, digest)| crate::ThemeDraftAsset {
                        path: path.into(),
                        mime: mime.into(),
                        bytes: bytes.to_vec(),
                        digest: common::theme::ThemeAssetDigest::from_digest(digest),
                    })
                    .collect(),
            },
            bindings: Vec::new(),
            header_pool: Vec::new(),
        }
    }

    fn media_pool_entry(actor: UserId) -> crate::ThemeHeaderPoolEntry {
        crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: None,
            media_user_id: Some(actor),
            media_source: Some("upload".into()),
            media_digest: Some("a".repeat(64).parse().unwrap()),
            media_filename: Some("header.png".into()),
        }
    }

    // guard:no-backend — strict empty mocks prove rejection precedes storage mutation.
    #[tokio::test]
    async fn editor_and_copy_reject_changed_package_and_foreign_media_before_writes() {
        let root = tempfile::tempdir().unwrap();
        let themes: Arc<dyn ThemeStorage> = Arc::new(crate::MockThemeStorage::new());
        let scope = crate::test_support::mock_write_scope();
        let assets = ThemeAssetManager::new(
            themes.clone(),
            scope.clone(),
            Arc::new(root.path().to_path_buf()),
        );
        let manager = ThemeManager::new(
            themes,
            Arc::new(crate::MockMediaStorage::new()),
            scope,
            Arc::new(MediaContentLocks::new(Arc::new(root.path().to_path_buf()))),
        );
        let actor = UserId::from(7);
        let snapshot = copy_fixture();
        let mut wrong_draft = snapshot.draft.clone();
        wrong_draft.stylesheet.push(b' ');
        assert!(
            manager
                .copy_saved(actor, ThemeOwner::Site, snapshot.clone(), wrong_draft)
                .await
                .unwrap_err()
                .to_string()
                .contains("validated package")
        );
        let mut foreign = snapshot.clone();
        foreign.header_pool.push(media_pool_entry(UserId::from(8)));
        assert!(
            manager
                .copy_saved(actor, ThemeOwner::Site, foreign.clone(), foreign.draft)
                .await
                .unwrap_err()
                .to_string()
                .contains("another user's Media")
        );
        for reason in ["context", "media", "pool"] {
            let mut snapshot = copy_fixture();
            snapshot.header_pool.push(media_pool_entry(actor));
            let (mut change, compiled) = editor_change(snapshot);
            match reason {
                "context" => change.draft.manifest.push(b' '),
                "media" => change.replacements.push(ThemeRoleBinding::Media {
                    theme_id: ThemeId::from(1),
                    role: ThemeImageRole::Logo,
                    user_id: UserId::from(8),
                    media: reference_from_fields(
                        Some("upload".into()),
                        Some("a".repeat(64).parse().unwrap()),
                        Some("header.png".into()),
                    )
                    .unwrap(),
                }),
                _ => change.replacements.push(ThemeRoleBinding::HeaderPool {
                    theme_id: ThemeId::from(1),
                    pool_revision: "a".repeat(64).parse().unwrap(),
                    shuffle_seed: [0; 32],
                }),
            }
            let error = manager
                .save_editor(
                    actor,
                    ThemeOwner::Site,
                    change,
                    &assets,
                    &compiled,
                    crate::ThemeQuotaLimits::production(),
                )
                .await
                .unwrap_err();
            let expected = match reason {
                "context" => "saved package context",
                "media" => "belong to the actor",
                _ => "not normal editor input",
            };
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    // guard:no-backend — complete owned pool identity passes through the transactional coordinator.
    #[tokio::test]
    async fn copy_preserves_owned_media_pool_identity_in_one_new_private_entry() {
        let actor = UserId::from(7);
        let mut snapshot = copy_fixture();
        snapshot.header_pool.push(media_pool_entry(actor));
        let expected = snapshot.clone();
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_copy_snapshot()
            .once()
            .returning(move |_, _, _| Ok(expected.clone()));
        themes
            .expect_create_theme_automatic()
            .once()
            .returning(|_, owner, _, _, _| {
                Ok(crate::ThemeCatalogEntry {
                    id: ThemeId::from(2),
                    owner,
                    name: "Source 2".into(),
                    current_revision: None,
                })
            });
        themes
            .expect_replace_header_pool()
            .once()
            .withf(move |_, owner, id, pool| {
                *owner == ThemeOwner::Site
                    && *id == ThemeId::from(2)
                    && pool == [media_pool_entry(actor)]
            })
            .returning(|_, _, _, _| Ok(()));
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_lock_media_reference()
            .once()
            .withf(|_, reference| reference.filename.as_ref() == "header.png")
            .returning(|_, _| Ok(()));
        let root = tempfile::tempdir().unwrap();
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(media),
            crate::test_support::mock_write_scope(),
            Arc::new(MediaContentLocks::new(Arc::new(root.path().to_path_buf()))),
        );
        let draft = snapshot.draft.clone();
        let entry = confirmed(
            manager
                .copy_saved(actor, ThemeOwner::Site, snapshot, draft)
                .await
                .unwrap(),
        );
        assert_eq!(entry.id, ThemeId::from(2));
        assert!(entry.current_revision.is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn editor_rejects_stale_saved_source_without_overwriting_newer_fields(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let compiled = compiled_theme_fixture();
        let id = create_site_theme(env.themes(), env.write_scope(), &compiled).await;
        let manager = ThemeManager::new(
            env.themes(),
            env.media(),
            env.write_scope(),
            Arc::new(env.media_content_locks()),
        );
        let snapshot = manager.copy_source(ThemeOwner::Site, id).await.unwrap();
        let original = snapshot.draft.clone();
        let (change, compiled) = editor_change(snapshot);
        let themes = env.themes();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .rename_theme(transaction, ThemeOwner::Site, id, "Newer name")
                        .await
                })
            })
            .await
            .unwrap();
        let assets = ThemeAssetManager::new(
            env.themes(),
            env.write_scope(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let error = manager
            .save_editor(
                UserId::from(0),
                ThemeOwner::Site,
                change,
                &assets,
                &compiled,
                crate::ThemeQuotaLimits::production(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("source changed"), "{error}");
        let after = manager.copy_source(ThemeOwner::Site, id).await.unwrap();
        assert_eq!(after.entry.name, "Newer name");
        assert_eq!(after.draft, original);
        assert!(after.bindings.is_empty());
        assert!(after.entry.current_revision.is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn editor_revalidates_selection_after_media_lock_wait(#[case] backend: Backend) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let id = published_site_theme(
            env.themes(),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let reference = seed_media(env.media(), env.write_scope(), actor, "editor-wait.png").await;
        let locks = Arc::new(env.media_content_locks());
        let manager = Arc::new(ThemeManager::new(
            env.themes(),
            env.media(),
            env.write_scope(),
            locks.clone(),
        ));
        manager
            .replace_role(
                actor,
                ThemeOwner::Site,
                id,
                ThemeImageRole::Logo,
                ThemeRoleInput::Media(reference.clone()),
            )
            .await
            .unwrap();
        let themes = env.themes();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .set_selection(
                            transaction,
                            ThemeOwner::Site,
                            Some(common::theme::PublicThemeSelection::Custom(id)),
                        )
                        .await
                })
            })
            .await
            .unwrap();
        let snapshot = manager.copy_source(ThemeOwner::Site, id).await.unwrap();
        let previous = env
            .themes()
            .list_revisions(ThemeOwner::Site, id)
            .await
            .unwrap();
        let (change, compiled) = editor_change(snapshot);
        let held = locks.acquire([&reference]).await.unwrap();
        let assets = Arc::new(ThemeAssetManager::new(
            env.themes(),
            env.write_scope(),
            Arc::new(env.base.path().to_path_buf()),
        ));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let started = barrier.clone();
        let save = tokio::spawn(async move {
            started.wait().await;
            manager
                .save_editor(
                    actor,
                    ThemeOwner::Site,
                    change,
                    &assets,
                    &compiled,
                    crate::ThemeQuotaLimits::production(),
                )
                .await
        });
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(!save.is_finished());
        let themes = env.themes();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .set_selection(
                            transaction,
                            ThemeOwner::Site,
                            Some(common::theme::PublicThemeSelection::BuiltIn(
                                common::theme::Theme::Reader,
                            )),
                        )
                        .await
                })
            })
            .await
            .unwrap();
        drop(held);
        let saved = confirmed(save.await.unwrap().unwrap());
        assert!(!saved.applied);
        assert_eq!(
            env.themes()
                .list_revisions(ThemeOwner::Site, id)
                .await
                .unwrap(),
            previous
        );
        assert_eq!(
            env.themes().selection(ThemeOwner::Site).await.unwrap(),
            Some(common::theme::PublicThemeSelection::BuiltIn(
                common::theme::Theme::Reader
            ))
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn editor_publication_quota_failure_rolls_back_all_fields_and_new_files(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let id = published_site_theme(
            env.themes(),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let manager = ThemeManager::new(
            env.themes(),
            env.media(),
            env.write_scope(),
            Arc::new(env.media_content_locks()),
        );
        let themes = env.themes();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .set_selection(
                            transaction,
                            ThemeOwner::Site,
                            Some(common::theme::PublicThemeSelection::Custom(id)),
                        )
                        .await
                })
            })
            .await
            .unwrap();
        manager
            .replace_header_pool(
                actor,
                ThemeOwner::Site,
                id,
                vec![ThemePoolInput::PackageAsset("assets/pixel.png".into())],
                [7; 32],
            )
            .await
            .unwrap();
        let before = manager.copy_source(ThemeOwner::Site, id).await.unwrap();
        assert!(!before.header_pool.is_empty());
        let eligibility = env.themes().list_content_eligibility().await.unwrap();
        let (mut change, compiled) = editor_change(before.clone());
        change.replacements.push(ThemeRoleBinding::ExplicitAbsent {
            theme_id: id,
            role: ThemeImageRole::Header,
        });
        let digest = common::theme::ThemeStylesheetDigest::from_digest(
            compiled.stylesheet_content().digest(),
        );
        let assets = ThemeAssetManager::new(
            env.themes(),
            env.write_scope(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let mut limits = crate::ThemeQuotaLimits::production();
        limits.retained_revisions = 0;
        assert!(
            manager
                .save_editor(actor, ThemeOwner::Site, change, &assets, &compiled, limits)
                .await
                .is_err()
        );
        assert_eq!(
            manager.copy_source(ThemeOwner::Site, id).await.unwrap(),
            before
        );
        assert_eq!(
            env.themes().list_content_eligibility().await.unwrap(),
            eligibility
        );
        let path = env
            .base
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4])
            .join(digest.as_ref());
        assert!(
            !path.exists(),
            "known rollback must clean only newly staged content"
        );
    }

    // guard:no-backend — injected acknowledgement loss proves one creation and one reconciliation, never replay.
    #[tokio::test]
    async fn editor_commit_acknowledgement_loss_remains_uncertain_without_replay() {
        for fail_read in [false, true] {
            let mut themes = crate::MockThemeStorage::new();
            themes
                .expect_create_theme()
                .once()
                .returning(|_, _, _, _, _| Ok(ThemeId::from(42)));
            themes.expect_get_draft().once().returning(move |_, _| {
                if fail_read {
                    Err(sqlx::Error::Protocol("reconciliation unavailable".into()))
                } else {
                    Ok(None)
                }
            });
            let root = tempfile::tempdir().unwrap();
            let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
            let scope = crate::test_support::mock_write_scope_with_commit_acknowledgement_loss();
            let assets = ThemeAssetManager::new(
                themes.clone(),
                scope.clone(),
                Arc::new(root.path().to_path_buf()),
            );
            let manager = ThemeManager::new(
                themes,
                Arc::new(crate::MockMediaStorage::new()),
                scope,
                Arc::new(MediaContentLocks::new(Arc::new(root.path().to_path_buf()))),
            );
            let draft = crate::ThemeDraft {
                theme_id: ThemeId::from(0),
                manifest: b"{}".to_vec(),
                stylesheet: Vec::new(),
                source_digest: "a".repeat(64).parse().unwrap(),
                assets: Vec::new(),
            };
            let outcome = manager
                .save_editor(
                    UserId::from(0),
                    ThemeOwner::Site,
                    ThemeEditorChange {
                        name: "Unknown new".into(),
                        expected: None,
                        draft,
                        replacements: Vec::new(),
                    },
                    &assets,
                    &compiled_theme_fixture(),
                    crate::ThemeQuotaLimits::production(),
                )
                .await
                .unwrap();
            assert!(
                matches!(outcome, MutationOutcome::CommitIndeterminate(ThemeEditorSaved {id, applied:false,..}) if id == ThemeId::from(42))
            );
        }
    }

    // guard:no-backend — lost acknowledgement must retain active staged bytes without repeating publication.
    #[tokio::test]
    async fn editor_active_commit_acknowledgement_loss_retains_staged_bytes() {
        let compiled = compiled_theme_fixture();
        let id = ThemeId::from(42);
        let draft = crate::ThemeDraft {
            theme_id: id,
            manifest: compiled.canonical_manifest().to_vec(),
            stylesheet: b"body { color: black; }".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: compiled
                .assets()
                .map(|(path, mime, bytes, digest)| crate::ThemeDraftAsset {
                    path: path.into(),
                    mime: mime.into(),
                    bytes: bytes.to_vec(),
                    digest: common::theme::ThemeAssetDigest::from_digest(digest),
                })
                .collect(),
        };
        let snapshot = crate::ThemeCopySnapshot {
            entry: crate::ThemeCatalogEntry {
                id,
                owner: ThemeOwner::Site,
                name: "Original".into(),
                current_revision: Some("b".repeat(64).parse().unwrap()),
            },
            draft,
            bindings: Vec::new(),
            header_pool: Vec::new(),
        };
        let (change, compiled) = editor_change(snapshot.clone());
        let digest = common::theme::ThemeStylesheetDigest::from_digest(
            compiled.stylesheet_content().digest(),
        );
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_copy_snapshot()
            .once()
            .returning(move |_, _, _| Ok(snapshot.clone()));
        themes
            .expect_selection_for_update()
            .once()
            .returning(move |_, _| Ok(Some(common::theme::PublicThemeSelection::Custom(id))));
        themes
            .expect_replace_draft()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_rename_theme()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_admit_publication()
            .once()
            .returning(|_, _| Ok(()));
        themes.expect_get_draft().once().returning(|_, _| Ok(None));
        let root = tempfile::tempdir().unwrap();
        let scope = crate::test_support::mock_write_scope_with_commit_acknowledgement_loss();
        let themes: Arc<dyn ThemeStorage> = Arc::new(themes);
        let assets = ThemeAssetManager::new(
            themes.clone(),
            scope.clone(),
            Arc::new(root.path().to_path_buf()),
        );
        let manager = ThemeManager::new(
            themes,
            Arc::new(crate::MockMediaStorage::new()),
            scope,
            Arc::new(MediaContentLocks::new(Arc::new(root.path().to_path_buf()))),
        );
        let outcome = manager
            .save_editor(
                UserId::from(0),
                ThemeOwner::Site,
                change,
                &assets,
                &compiled,
                crate::ThemeQuotaLimits::production(),
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            MutationOutcome::CommitIndeterminate(ThemeEditorSaved { applied: false, .. })
        ));
        let path = root
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4])
            .join(digest.as_ref());
        assert_eq!(std::fs::read(path).unwrap(), compiled.css().bytes());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn copy_revalidates_saved_source_after_media_lock_wait(#[case] backend: Backend) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let media = seed_media(env.media(), env.write_scope(), actor, "copy-lock.png").await;
        let source =
            create_site_theme(env.themes(), env.write_scope(), &compiled_theme_fixture()).await;
        let locks = Arc::new(env.media_content_locks());
        let manager = Arc::new(ThemeManager::new(
            env.themes(),
            env.media(),
            env.write_scope(),
            locks.clone(),
        ));
        manager
            .replace_role(
                actor,
                ThemeOwner::Site,
                source,
                ThemeImageRole::Logo,
                ThemeRoleInput::Media(media.clone()),
            )
            .await
            .unwrap();
        let snapshot = manager.copy_source(ThemeOwner::Site, source).await.unwrap();
        let draft = snapshot.draft.clone();
        let held = locks.acquire([&media]).await.unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let waiter_barrier = barrier.clone();
        let copy = tokio::spawn(async move {
            waiter_barrier.wait().await;
            manager
                .copy_saved(actor, ThemeOwner::Site, snapshot, draft)
                .await
        });
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(!copy.is_finished());
        let themes = env.themes();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .rename_theme(transaction, ThemeOwner::Site, source, "Changed source")
                        .await
                })
            })
            .await
            .unwrap();
        drop(held);
        let error = copy.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("source changed"), "{error}");
        let entries = env.themes().list_themes(ThemeOwner::Site).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Changed source");
    }

    // guard:no-backend — injected commit acknowledgement loss proves no automatic Copy replay.
    #[tokio::test]
    async fn copy_commit_acknowledgement_loss_returns_uncertainty_without_replay() {
        let snapshot = crate::ThemeCopySnapshot {
            entry: crate::ThemeCatalogEntry {
                id: ThemeId::from(1),
                owner: ThemeOwner::Site,
                name: "Source".into(),
                current_revision: None,
            },
            draft: crate::ThemeDraft {
                theme_id: ThemeId::from(1),
                manifest: b"{}".to_vec(),
                stylesheet: b"body{}".to_vec(),
                source_digest: "a".repeat(64).parse().unwrap(),
                assets: Vec::new(),
            },
            bindings: Vec::new(),
            header_pool: Vec::new(),
        };
        let mut themes = crate::MockThemeStorage::new();
        let expected = snapshot.clone();
        themes
            .expect_copy_snapshot()
            .once()
            .returning(move |_, _, _| Ok(expected.clone()));
        themes
            .expect_create_theme_automatic()
            .once()
            .returning(|_, owner, _, _, _| {
                Ok(crate::ThemeCatalogEntry {
                    id: ThemeId::from(2),
                    owner,
                    name: "Source 2".into(),
                    current_revision: None,
                })
            });
        let root = tempfile::tempdir().unwrap();
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(MediaContentLocks::new(Arc::new(root.path().to_path_buf()))),
        );
        let draft = snapshot.draft.clone();
        let result = manager
            .copy_saved(UserId::from(0), ThemeOwner::Site, snapshot, draft)
            .await
            .unwrap();
        assert!(
            matches!(result, MutationOutcome::CommitIndeterminate(entry) if entry.id == ThemeId::from(2) && entry.name == "Source 2")
        );
    }

    // guard:no-backend — acknowledgement-loss confirmation is a write-scope contract
    #[tokio::test]
    async fn role_commit_acknowledgement_loss_rechecks_the_persisted_binding() {
        let theme_id = ThemeId::from(9);
        let binding = ThemeRoleBinding::PackagedDefault {
            theme_id,
            role: ThemeImageRole::Logo,
        };
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_role_binding()
            .times(3)
            .returning(move |_, _, role| {
                Ok((role == ThemeImageRole::Logo).then_some(binding.clone()))
            });
        themes
            .expect_header_pool()
            .once()
            .returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .once()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_replace_role_binding()
            .once()
            .returning(|_, _, _| Ok(()));
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(
                lock_root.path().to_path_buf(),
            ))),
        );

        assert!(matches!(
            manager
                .replace_role(
                    UserId::from(0),
                    ThemeOwner::Site,
                    theme_id,
                    ThemeImageRole::Logo,
                    ThemeRoleInput::PackagedDefault,
                )
                .await
                .expect("persisted binding confirms acknowledgement loss"),
            MutationOutcome::Confirmed(())
        ));
    }
    // guard:no-backend — snapshot decoding is isolated from database mechanics
    #[tokio::test]
    async fn media_snapshot_includes_complete_pool_media_once() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_role_binding().returning(|_, _, _| Ok(None));
        themes.expect_header_pool().once().returning(|_, _| {
            Ok(vec![ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: None,
                media_user_id: Some(UserId::from(1)),
                media_source: Some("upload".to_owned()),
                media_digest: Some("a".repeat(64).parse().unwrap()),
                media_filename: Some("hero.png".to_owned()),
            }])
        });
        let references = snapshot_for(&themes, ThemeOwner::Site, theme_id)
            .await
            .expect("complete pool media decodes");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].filename.as_ref(), "hero.png");
    }

    // guard:no-backend — snapshot decoding is isolated from database mechanics
    #[tokio::test]
    async fn media_snapshot_rejects_incomplete_pool_media() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_role_binding().returning(|_, _, _| Ok(None));
        themes.expect_header_pool().once().returning(|_, _| {
            Ok(vec![ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: None,
                media_user_id: Some(UserId::from(1)),
                media_source: None,
                media_digest: None,
                media_filename: None,
            }])
        });

        assert!(
            snapshot_for(&themes, ThemeOwner::Site, theme_id)
                .await
                .expect_err("incomplete pool media must be rejected")
                .to_string()
                .contains("missing source")
        );
    }
    // guard:no-backend — shuffle confirmation depends only on the persisted seed
    #[tokio::test]
    async fn shuffle_acknowledgement_loss_confirms_the_reloaded_seed() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_shuffle_header_pool()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_role_binding()
            .once()
            .returning(move |_, _, _| {
                Ok(Some(ThemeRoleBinding::HeaderPool {
                    theme_id,
                    pool_revision: "a".repeat(64).parse().unwrap(),
                    shuffle_seed: [7; 32],
                }))
            });
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(
                lock_root.path().to_path_buf(),
            ))),
        );
        assert!(matches!(
            manager
                .shuffle_header_pool(UserId::from(0), ThemeOwner::Site, theme_id, [7; 32])
                .await
                .expect("matching persisted seed confirms acknowledgement loss"),
            MutationOutcome::Confirmed(())
        ));
    }

    // guard:no-backend — acknowledgement-loss confirmation must preserve uncertainty on mismatch
    #[tokio::test]
    async fn shuffle_acknowledgement_loss_preserves_uncertainty_for_a_different_seed() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_shuffle_header_pool()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_role_binding()
            .once()
            .returning(move |_, _, _| {
                Ok(Some(ThemeRoleBinding::HeaderPool {
                    theme_id,
                    pool_revision: "a".repeat(64).parse().unwrap(),
                    shuffle_seed: [0; 32],
                }))
            });
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(
                lock_root.path().to_path_buf(),
            ))),
        );
        assert!(matches!(
            manager
                .shuffle_header_pool(UserId::from(0), ThemeOwner::Site, theme_id, [7; 32])
                .await
                .expect("a mismatched seed remains indeterminate"),
            MutationOutcome::CommitIndeterminate(())
        ));
    }

    // guard:no-backend — acknowledgement-loss confirmation is a write-scope contract
    #[tokio::test]
    async fn header_pool_commit_acknowledgement_loss_rechecks_binding_and_entries() {
        let theme_id = ThemeId::from(9);
        let (pool, expected_entries, _) = ThemeManager::pool(
            UserId::from(0),
            vec![ThemePoolInput::PackageAsset("header.png".to_owned())],
        )
        .expect("complete package pool");
        let expected_binding = ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision: pool.revision().clone(),
            shuffle_seed: [7; 32],
        };
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_role_binding()
            .times(3)
            .returning(move |_, _, role| {
                Ok((role == ThemeImageRole::Header).then_some(expected_binding.clone()))
            });
        let entries_for_reads = expected_entries.clone();
        themes
            .expect_header_pool()
            .times(2)
            .returning(move |_, _| Ok(entries_for_reads.clone()));
        themes
            .expect_locked_media_references()
            .once()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_replace_header_pool()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_replace_role_binding()
            .once()
            .returning(|_, _, _| Ok(()));
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(
                lock_root.path().to_path_buf(),
            ))),
        );

        assert!(matches!(
            manager
                .replace_header_pool(
                    UserId::from(0),
                    ThemeOwner::Site,
                    theme_id,
                    vec![ThemePoolInput::PackageAsset("header.png".to_owned())],
                    [7; 32],
                )
                .await
                .expect("persisted pool confirms acknowledgement loss"),
            MutationOutcome::Confirmed(())
        ));
    }

    // guard:no-backend — acknowledgement-loss confirmation is a write-scope contract
    #[tokio::test]
    async fn remove_commit_acknowledgement_loss_rechecks_catalog_membership() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_role_binding().returning(|_, _, _| Ok(None));
        themes
            .expect_header_pool()
            .once()
            .returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .once()
            .returning(|_, _, _| Ok(Vec::new()));
        themes
            .expect_remove_theme()
            .once()
            .returning(|_, _, _, _| Ok(()));
        themes
            .expect_list_themes()
            .once()
            .returning(|_| Ok(Vec::new()));
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        let manager = ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(
                lock_root.path().to_path_buf(),
            ))),
        );

        assert!(matches!(
            manager
                .remove_theme(UserId::from(0), ThemeOwner::Site, theme_id, 10)
                .await
                .expect("missing persisted theme confirms acknowledgement loss"),
            MutationOutcome::Confirmed(())
        ));
    }

    fn changed_media_reference() -> crate::ThemeMediaReference {
        crate::ThemeMediaReference {
            user_id: UserId::from(1),
            media: common::media::MediaRef {
                source: "upload".parse().unwrap(),
                sha256: "a".repeat(64).parse().unwrap(),
                filename: "changed.png".parse().unwrap(),
            },
        }
    }

    fn manager_with_changed_snapshot() -> ThemeManager {
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_role_binding()
            .times(2)
            .returning(|_, _, _| Ok(None));
        themes
            .expect_header_pool()
            .once()
            .returning(|_, _| Ok(Vec::new()));
        themes
            .expect_locked_media_references()
            .once()
            .returning(|_, _, _| Ok(vec![changed_media_reference()]));
        let lock_root = tempfile::TempDir::new().expect("create lock root");
        ThemeManager::new(
            Arc::new(themes),
            Arc::new(crate::MockMediaStorage::new()),
            crate::test_support::mock_write_scope(),
            Arc::new(crate::MediaContentLocks::new(Arc::new(lock_root.keep()))),
        )
    }

    // guard:no-backend — mocks deterministically change the aggregate between snapshots.
    #[tokio::test]
    async fn replace_role_rejects_a_changed_locked_media_snapshot() {
        assert!(
            manager_with_changed_snapshot()
                .replace_role(
                    UserId::from(0),
                    ThemeOwner::Site,
                    ThemeId::from(9),
                    ThemeImageRole::Logo,
                    ThemeRoleInput::PackagedDefault,
                )
                .await
                .expect_err("changed binding must reject")
                .to_string()
                .contains("changed while acquiring locks")
        );
    }

    // guard:no-backend — mocks deterministically change the aggregate between snapshots.
    #[tokio::test]
    async fn replace_header_pool_rejects_a_changed_locked_media_snapshot() {
        assert!(
            manager_with_changed_snapshot()
                .replace_header_pool(
                    UserId::from(0),
                    ThemeOwner::Site,
                    ThemeId::from(9),
                    vec![ThemePoolInput::PackageAsset("header.png".to_owned())],
                    [0; 32],
                )
                .await
                .expect_err("changed binding must reject")
                .to_string()
                .contains("changed while acquiring locks")
        );
    }

    // guard:no-backend — mocks deterministically change the aggregate between snapshots.
    #[tokio::test]
    async fn remove_theme_rejects_a_changed_locked_media_snapshot() {
        assert!(
            manager_with_changed_snapshot()
                .remove_theme(UserId::from(0), ThemeOwner::Site, ThemeId::from(9), 10)
                .await
                .expect_err("changed binding must reject")
                .to_string()
                .contains("changed while acquiring locks")
        );
    }

    async fn published_site_theme(
        themes: Arc<dyn ThemeStorage>,
        write_scope: crate::WriteScope,
        media_root: std::path::PathBuf,
    ) -> common::ids::ThemeId {
        let compiled = compiled_theme_fixture();
        let theme_id = create_site_theme(Arc::clone(&themes), write_scope.clone(), &compiled).await;
        let content_bytes = compiled
            .css()
            .bytes()
            .len()
            .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
            .and_then(|bytes| i64::try_from(bytes).ok())
            .expect("fixture content bytes fit");
        let manager = ThemeAssetManager::new(themes, write_scope, Arc::new(media_root));
        confirmed(
            manager
                .publish(
                    ThemeOwner::Site,
                    theme_id,
                    &compiled,
                    theme_quota_limits(content_bytes),
                    100,
                )
                .await
                .expect("publish fixture theme"),
        );
        theme_id
    }

    async fn published_author_theme(
        themes: Arc<dyn ThemeStorage>,
        write_scope: crate::WriteScope,
        media_root: std::path::PathBuf,
        author: UserId,
    ) -> common::ids::ThemeId {
        let compiled = compiled_theme_fixture();
        let draft = ThemeDraft {
            theme_id: ThemeId::from(0),
            manifest: compiled.canonical_manifest().to_vec(),
            stylesheet: b"body { color: black; }".to_vec(),
            source_digest: "a".repeat(64).parse().unwrap(),
            assets: Vec::new(),
        };
        let create_themes = Arc::clone(&themes);
        let theme_id = confirmed(
            write_scope
                .clone()
                .run(move |transaction| {
                    Box::pin(async move {
                        create_themes
                            .create_theme(
                                transaction,
                                ThemeOwner::Author(author),
                                "Author Test",
                                &draft,
                                theme_quota_limits(i64::MAX),
                            )
                            .await
                    })
                })
                .await
                .expect("create author theme"),
        );
        let content_bytes = compiled
            .css()
            .bytes()
            .len()
            .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
            .and_then(|bytes| i64::try_from(bytes).ok())
            .expect("fixture content bytes fit");
        let manager = ThemeAssetManager::new(themes, write_scope, Arc::new(media_root));
        confirmed(
            manager
                .publish(
                    ThemeOwner::Author(author),
                    theme_id,
                    &compiled,
                    theme_quota_limits(content_bytes),
                    100,
                )
                .await
                .expect("publish author theme"),
        );
        theme_id
    }

    #[apply(backends)]
    #[tokio::test]
    async fn manager_binds_actor_media_and_preserves_valid_bindings_on_rejection(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let other = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            actor,
            "theme-logo.png",
        )
        .await;
        let theme_id = published_site_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .replace_role(
                    actor,
                    ThemeOwner::Site,
                    theme_id,
                    ThemeImageRole::Logo,
                    ThemeRoleInput::Media(media.clone()),
                )
                .await
                .expect("actor media binding succeeds"),
        );
        let media_binding = env
            .themes()
            .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Logo)
            .await
            .expect("read media binding")
            .expect("media binding exists");
        assert!(matches!(
            media_binding,
            ThemeRoleBinding::Media { user_id, media: ref bound, .. }
                if user_id == actor && bound == &media
        ));

        assert!(
            manager
                .replace_role(
                    other,
                    ThemeOwner::Author(actor),
                    theme_id,
                    ThemeImageRole::Logo,
                    ThemeRoleInput::Media(media.clone()),
                )
                .await
                .is_err(),
            "a different author cannot mutate an author's catalog"
        );
        assert!(
            manager
                .replace_role(
                    actor,
                    ThemeOwner::Site,
                    theme_id,
                    ThemeImageRole::Logo,
                    ThemeRoleInput::PackageAsset("assets/removed.png".into()),
                )
                .await
                .is_err(),
            "a path absent from the current revision is rejected"
        );
        assert_eq!(
            env.themes()
                .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Logo)
                .await
                .expect("read binding after rejected replacement"),
            Some(media_binding),
            "a rejected candidate does not change the current binding"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn manager_canonicalizes_mixed_pool_and_shuffles_only_its_seed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            actor,
            "theme-header.png",
        )
        .await;
        let theme_id = published_site_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .replace_header_pool(
                    actor,
                    ThemeOwner::Site,
                    theme_id,
                    vec![
                        ThemePoolInput::Media(media),
                        ThemePoolInput::PackageAsset("assets/pixel.png".into()),
                    ],
                    [1; 32],
                )
                .await
                .expect("mixed pool replacement succeeds"),
        );
        let initial = env
            .themes()
            .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Header)
            .await
            .expect("read pool binding")
            .expect("pool binding exists");
        let pool = env
            .themes()
            .header_pool(ThemeOwner::Site, theme_id)
            .await
            .expect("read canonical pool");
        assert!(matches!(
            &initial,
            ThemeRoleBinding::HeaderPool {
                shuffle_seed,
                ..
            } if *shuffle_seed == [1; 32]
        ));
        assert_eq!(pool.len(), 2);
        assert_eq!(
            pool.iter()
                .filter_map(|entry| entry.media_user_id)
                .collect::<Vec<_>>(),
            vec![actor]
        );

        confirmed(
            manager
                .shuffle_header_pool(actor, ThemeOwner::Site, theme_id, [2; 32])
                .await
                .expect("shuffle succeeds"),
        );
        let shuffled = env
            .themes()
            .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Header)
            .await
            .expect("read shuffled binding")
            .expect("pool binding remains");
        assert!(matches!(
            (&initial, &shuffled),
            (
                ThemeRoleBinding::HeaderPool {
                    pool_revision: initial_revision,
                    ..
                },
                ThemeRoleBinding::HeaderPool {
                    pool_revision: shuffled_revision,
                    shuffle_seed,
                    ..
                }
            ) if initial_revision == shuffled_revision && *shuffle_seed == [2; 32]
        ));
        assert_eq!(
            env.themes()
                .header_pool(ThemeOwner::Site, theme_id)
                .await
                .expect("read pool after shuffle"),
            pool,
            "shuffle changes no pool entry"
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn manager_removes_site_theme_to_studio_and_retains_its_content(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let theme_id = published_site_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Site,
                                Some(common::theme::PublicThemeSelection::Custom(theme_id)),
                            )
                            .await
                    })
                })
                .await
                .expect("select custom site theme"),
        );
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(UserId::from(0), ThemeOwner::Site, theme_id, 200)
                .await
                .expect("remove published site theme"),
        );
        assert_eq!(
            env.themes()
                .selection(ThemeOwner::Site)
                .await
                .expect("read reset selection"),
            Some(common::theme::PublicThemeSelection::BuiltIn(
                common::theme::Theme::Studio
            ))
        );
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read site catalog")
                .is_empty()
        );
        assert!(
            env.themes()
                .list_content_eligibility()
                .await
                .expect("read retained content")
                .iter()
                .all(|content| content.retained_until_unix_seconds == 200),
            "removed revision content remains eligible until retention"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn removing_unselected_site_theme_preserves_site_selection(#[case] backend: Backend) {
        let env = backend.setup().await;
        let theme_id = published_site_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Site,
                                Some(common::theme::PublicThemeSelection::BuiltIn(
                                    common::theme::Theme::Terminal,
                                )),
                            )
                            .await
                    })
                })
                .await
                .expect("select unrelated built-in site theme"),
        );
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(UserId::from(0), ThemeOwner::Site, theme_id, 200)
                .await
                .expect("remove unselected site theme"),
        );
        assert_eq!(
            env.themes()
                .selection(ThemeOwner::Site)
                .await
                .expect("read preserved selection"),
            Some(common::theme::PublicThemeSelection::BuiltIn(
                common::theme::Theme::Terminal
            ))
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn manager_removes_author_override_to_inheritance(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let theme_id = published_author_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
            author,
        )
        .await;
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Author(author),
                                Some(common::theme::PublicThemeSelection::Custom(theme_id)),
                            )
                            .await
                    })
                })
                .await
                .expect("select author override"),
        );
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(author, ThemeOwner::Author(author), theme_id, 200)
                .await
                .expect("remove author theme"),
        );
        assert_eq!(
            env.themes()
                .selection(ThemeOwner::Author(author))
                .await
                .expect("read author selection"),
            None
        );
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Author(author))
                .await
                .expect("read author catalog")
                .is_empty()
        );
    }

    #[test]
    fn binding_and_reference_conversion_preserve_complete_media_identity() {
        let media = common::media::MediaRef {
            source: "upload".parse().unwrap(),
            sha256: "b".repeat(64).parse().unwrap(),
            filename: "hero.png".parse().unwrap(),
        };
        assert!(matches!(
            ThemeManager::binding(
                UserId::from(1),
                ThemeId::from(9),
                ThemeImageRole::Logo,
                ThemeRoleInput::PackagedDefault,
            ),
            ThemeRoleBinding::PackagedDefault { .. }
        ));
        let restored = reference_from_fields(
            Some(media.source.to_string()),
            Some(media.sha256.to_string().parse().unwrap()),
            Some(media.filename.to_string()),
        )
        .expect("complete stored fields restore their exact media reference");
        assert_eq!(restored, media);
        assert!(
            reference_from_fields(None, None, None)
                .expect_err("partial stored fields are corrupt")
                .to_string()
                .contains("missing source")
        );
    }

    #[test]
    fn scope_error_preserves_write_scope_begin_failures() {
        assert!(matches!(
            scope_error(WriteScopeError::Begin(sqlx::Error::RowNotFound)),
            error if error.downcast_ref::<sqlx::Error>().is_some_and(|error| matches!(
                error,
                sqlx::Error::RowNotFound
            ))
        ));
    }

    #[test]
    fn media_sorting_and_invalid_pool_rows_are_deterministic() {
        let first = common::media::MediaRef {
            source: "upload".parse().unwrap(),
            sha256: "a".repeat(64).parse().unwrap(),
            filename: "a.png".parse().unwrap(),
        };
        let second = common::media::MediaRef {
            source: "upload".parse().unwrap(),
            sha256: "b".repeat(64).parse().unwrap(),
            filename: "b.png".parse().unwrap(),
        };
        assert!(media_sort_key(&first) < media_sort_key(&second));
        assert_eq!(
            header_entry_encoding(&ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: Some("logo.png".to_owned()),
                media_user_id: Some(UserId::from(1)),
                media_source: None,
                media_digest: None,
                media_filename: None,
            }),
            Vec::<u8>::new()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publication_rejects_package_binding_missing_from_candidate_revision(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let actor = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let theme_id = published_site_theme(
            Arc::clone(&env.themes()),
            env.write_scope(),
            env.base.path().to_path_buf(),
        )
        .await;
        let manager = ThemeManager::new(
            Arc::clone(&env.themes()),
            Arc::clone(&env.media()),
            env.write_scope().clone(),
            Arc::new(env.media_content_locks()),
        );
        confirmed(
            manager
                .replace_role(
                    actor,
                    ThemeOwner::Site,
                    theme_id,
                    ThemeImageRole::Logo,
                    ThemeRoleInput::PackageAsset("assets/pixel.png".to_owned()),
                )
                .await
                .expect("bind current package asset"),
        );
        let current = env
            .themes()
            .list_themes(ThemeOwner::Site)
            .await
            .expect("read current theme")[0]
            .current_revision
            .clone();
        let revision = ThemeRevision {
            theme_id,
            digest: "d".repeat(64).parse().unwrap(),
            stylesheet_digest: "e".repeat(64).parse().unwrap(),
            manifest: b"{}".to_vec(),
        };
        let themes = Arc::clone(&env.themes());
        let result = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .admit_publication(
                            transaction,
                            ThemePublicationAdmission {
                                owner: ThemeOwner::Site,
                                limits: theme_quota_limits(i64::MAX),
                                revision: &revision,
                                assets: &[],
                                eligibilities: &[],
                                charges: &[],
                            },
                        )
                        .await
                })
            })
            .await;
        assert!(matches!(
            result,
            Err(WriteScopeError::Operation(sqlx::Error::RowNotFound))
        ));
        assert_eq!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read theme after rejected publication")[0]
                .current_revision,
            current
        );
    }
}
