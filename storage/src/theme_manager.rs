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
        &entry.media_source,
        &entry.media_digest,
        &entry.media_filename,
    ) {
        (Some(path), None, None, None) => {
            ThemePoolEntry::Package(path.clone()).canonical_encoding()
        }
        (None, Some(source), Some(digest), Some(filename)) => ThemePoolEntry::Media {
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

    async fn published_site_theme(env: &crate::test_support::TestEnv) -> common::ids::ThemeId {
        let compiled = compiled_theme_fixture();
        let theme_id = create_site_theme(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            &compiled,
        )
        .await;
        let content_bytes = compiled
            .css()
            .bytes()
            .len()
            .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
            .and_then(|bytes| i64::try_from(bytes).ok())
            .expect("fixture content bytes fit");
        let manager = ThemeAssetManager::new(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
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
        env: &crate::test_support::TestEnv,
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
        let themes = Arc::clone(&env.state.themes);
        let theme_id = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
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
        let manager = ThemeAssetManager::new(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
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
        let actor = SeedUser::new().seed(&env.state).await.user_id;
        let other = SeedUser::new().seed(&env.state).await.user_id;
        let media = seed_media(&env.state, actor, "theme-logo.png").await;
        let theme_id = published_site_theme(&env).await;
        let manager = ThemeManager::new(
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
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
            .state
            .themes
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
            env.state
                .themes
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
        let actor = SeedUser::new().seed(&env.state).await.user_id;
        let media = seed_media(&env.state, actor, "theme-header.png").await;
        let theme_id = published_site_theme(&env).await;
        let manager = ThemeManager::new(
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
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
            .state
            .themes
            .role_binding(ThemeOwner::Site, theme_id, ThemeImageRole::Header)
            .await
            .expect("read pool binding")
            .expect("pool binding exists");
        let pool = env
            .state
            .themes
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
            .state
            .themes
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
            env.state
                .themes
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
        let theme_id = published_site_theme(&env).await;
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
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
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(UserId::from(0), ThemeOwner::Site, theme_id, 200)
                .await
                .expect("remove published site theme"),
        );
        assert_eq!(
            env.state
                .themes
                .selection(ThemeOwner::Site)
                .await
                .expect("read reset selection"),
            Some(common::theme::PublicThemeSelection::BuiltIn(
                common::theme::Theme::Studio
            ))
        );
        assert!(
            env.state
                .themes
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read site catalog")
                .is_empty()
        );
        assert!(
            env.state
                .themes
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
        let theme_id = published_site_theme(&env).await;
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
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
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(UserId::from(0), ThemeOwner::Site, theme_id, 200)
                .await
                .expect("remove unselected site theme"),
        );
        assert_eq!(
            env.state
                .themes
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
        let author = SeedUser::new().seed(&env.state).await.user_id;
        let theme_id = published_author_theme(&env, author).await;
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
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
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
            Arc::new(env.media_content_locks()),
        );

        confirmed(
            manager
                .remove_theme(author, ThemeOwner::Author(author), theme_id, 200)
                .await
                .expect("remove author theme"),
        );
        assert_eq!(
            env.state
                .themes
                .selection(ThemeOwner::Author(author))
                .await
                .expect("read author selection"),
            None
        );
        assert!(
            env.state
                .themes
                .list_themes(ThemeOwner::Author(author))
                .await
                .expect("read author catalog")
                .is_empty()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publication_rejects_package_binding_missing_from_candidate_revision(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let actor = SeedUser::new().seed(&env.state).await.user_id;
        let theme_id = published_site_theme(&env).await;
        let manager = ThemeManager::new(
            Arc::clone(&env.state.themes),
            Arc::clone(&env.state.media),
            env.state.write_scope.clone(),
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
            .state
            .themes
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
        let themes = Arc::clone(&env.state.themes);
        let result = env
            .state
            .write_scope
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
            env.state
                .themes
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read theme after rejected publication")[0]
                .current_revision,
            current
        );
    }
}
