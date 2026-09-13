//! Deterministic performance-fixture population through the public storage seam.
//!
//! This module is deliberately owned by `test-support`: it links the same typed
//! storage services as the application but is structurally absent from production.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, bail};
use common::{
    ids::{PostId, UserId},
    media::{
        ByteSize, ContentHash, Filename, MediaRef, MediaSource, detect_content_type,
        path as media_path, url as media_url,
    },
    pagination::PageSize,
    post_body::PostBody,
    post_title::PostTitle,
    seed::TimelineOrder,
    slug::Slug,
    tag::TagLabel,
    time::UtcInstant,
    username::Username,
    visibility::{AudienceTarget, ViewerIdentity},
};
use jiff::ToSpan as _;
use performance::{
    BrowserInitialRows, CountOverrides, Cursor, DatasetManifest, DatasetProfile, HistoryCursor,
    PersistedCursor, TimelineCursor, Workload, WorkloadSubjects, canonical_plan, plan,
    validate_manifest,
};
use sha2::Digest as _;
use storage::{
    AudienceStorage, MediaRecord, MediaStorage, OperatorStatus, PostBookkeepingExpectation,
    PostFormat, PostLifecycle, PostRevisionCursor, PostStorage, PublishedPageRequest,
    RenderedPostContent, SubscriptionStorage, UserStorage, WriteScope, render_post_input,
};

const FIXED_CLOCK: &str = "2026-09-01T12:00:00Z";
const SCHEDULED_OFFSET_HOURS: i64 = 876_000;
const FIXTURE_PASSWORD: &str = "performance-fixture-password";
const POST_BATCH_SIZE: usize = 256;
const MEDIA_BLOBS: [&[u8]; 5] = [
    b"performance fixture media 0\n",
    b"performance fixture media 1\n",
    b"performance fixture media 2\n",
    b"performance fixture media 3\n",
    b"performance fixture media 4\n",
];

/// Storage dependencies for one performance fixture population.
pub struct PerformanceSeedStorage {
    pub users: Arc<dyn UserStorage>,
    pub posts: Arc<dyn PostStorage>,
    pub subscriptions: Arc<dyn SubscriptionStorage>,
    pub audiences: Arc<dyn AudienceStorage>,
    pub media: Arc<dyn MediaStorage>,
    pub write_scope: WriteScope,
}

/// Independent counts captured after a confirmed write and from subsequent typed reads.
///
/// The keys are stable fixture-contract paths (for example, `lifecycle.live`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PerformanceSeedAudit {
    pub confirmed: BTreeMap<String, u64>,
    pub persisted: BTreeMap<String, u64>,
}

impl PerformanceSeedAudit {
    fn record_confirmed(&mut self, key: impl Into<String>) {
        *self.confirmed.entry(key.into()).or_default() += 1;
    }

    fn record_persisted(&mut self, key: impl Into<String>) {
        *self.persisted.entry(key.into()).or_default() += 1;
    }

    fn verify(&self) -> anyhow::Result<()> {
        if self.confirmed != self.persisted {
            bail!(
                "performance fixture confirmed and persisted audits differ: confirmed={:?}, persisted={:?}",
                self.confirmed,
                self.persisted
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct RevisionTarget {
    post_id: PostId,
    author: UserId,
    revisions: u64,
    deleted: bool,
}

#[derive(Clone, Copy)]
struct SeededPost {
    id: PostId,
    index: usize,
}

struct PersistedPostObservation {
    format: PostFormat,
    body_len: usize,
    tag_count: usize,
    audiences: Vec<AudienceTarget>,
    media_count: usize,
}

/// Receipt printed by the command after the manifest has been atomically published.
#[derive(serde::Serialize)]
pub struct PerformanceSeedReceipt<'a> {
    pub manifest_path: &'a Path,
    pub seeding_duration_us: u64,
}

/// Populate a fresh database and write its authoritative workload manifest.
///
/// # Errors
///
/// Returns an error when fixture creation, manifest validation, or atomic publication fails.
pub async fn seed_performance_fixture(
    storage: PerformanceSeedStorage,
    profile: DatasetProfile,
    overrides: CountOverrides,
    output: &Path,
    storage_path: &Path,
) -> anyhow::Result<(DatasetManifest, u64)> {
    let (manifest, _, duration) =
        seed_performance_fixture_with_audit(storage, profile, overrides, output, storage_path)
            .await?;
    Ok((manifest, duration))
}

/// Populate a fixture and retain mutation/persistence evidence for integration tests.
///
/// # Errors
///
/// Returns an error when fixture creation, audit verification, manifest validation, or atomic
/// publication fails.
pub async fn seed_performance_fixture_with_audit(
    storage: PerformanceSeedStorage,
    profile: DatasetProfile,
    overrides: CountOverrides,
    output: &Path,
    storage_path: &Path,
) -> anyhow::Result<(DatasetManifest, PerformanceSeedAudit, u64)> {
    let started = Instant::now();
    let plan = plan(profile, overrides).context("invalid performance fixture counts")?;
    let clock: UtcInstant = FIXED_CLOCK
        .parse()
        .context("fixed fixture clock is invalid")?;
    let authors = create_authors(&storage, plan.authors).await?;
    let audience_ids = create_audiences(&storage, &authors).await?;
    create_follows(&storage, &authors, plan.follows_per_author).await?;
    populate_audience_memberships(&storage, &authors, &audience_ids).await?;
    let media = create_media(&storage, storage_path, &authors, clock).await?;
    let mut audit = PerformanceSeedAudit::default();
    let posts = create_posts(
        &storage,
        &plan,
        &authors,
        &audience_ids,
        &media,
        clock,
        &mut audit,
    )
    .await?;
    apply_revisions(&storage, &plan, &authors, &posts, clock, &mut audit).await?;
    let persisted = audit_persisted(&storage, &plan, &authors, &posts, clock).await?;
    audit.persisted = persisted;
    audit.verify()?;
    let post_ids = posts.iter().map(|post| post.id).collect::<Vec<_>>();
    let manifest = resolve_manifest(&storage.posts, &plan, &authors, &post_ids, clock).await?;
    validate_manifest(&manifest).context("resolved performance manifest is invalid")?;
    write_manifest(output, &manifest)?;
    Ok((
        manifest,
        audit,
        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
    ))
}

async fn create_authors(
    storage: &PerformanceSeedStorage,
    count: u64,
) -> anyhow::Result<Vec<(Username, UserId)>> {
    let password = Arc::new(storage::prepare_password(FIXTURE_PASSWORD.parse()?).await?);
    let mut authors = Vec::with_capacity(usize::try_from(count)?);
    for index in 0..count {
        let username: Username = format!("perf-author-{index:04}").parse()?;
        let create = username.clone();
        let users = Arc::clone(&storage.users);
        let password = Arc::clone(&password);
        let outcome = storage
            .write_scope
            .run(move |tx| {
                Box::pin(async move {
                    users
                        .create_user(tx, &create, &password, None, OperatorStatus::STANDARD)
                        .await
                })
            })
            .await?;
        let id = crate::confirmed_fixture_outcome(outcome, "performance author creation")?;
        authors.push((username, id));
    }
    Ok(authors)
}

async fn create_follows(
    storage: &PerformanceSeedStorage,
    authors: &[(Username, UserId)],
    per_author: u64,
) -> anyhow::Result<()> {
    let channel = storage.subscriptions.local_channel_id().await?;
    let per_author = usize::try_from(per_author)?;
    for (index, (_, author)) in authors.iter().enumerate() {
        let subscriptions = Arc::clone(&storage.subscriptions);
        let subscribers = authors
            .iter()
            .cycle()
            .skip(index + 1)
            .take(per_author)
            .map(|(_, id)| common::visibility::local_subscriber_identity(channel, *id))
            .collect::<Vec<_>>();
        let author = *author;
        let outcome = storage
            .write_scope
            .run(move |tx| {
                Box::pin(async move {
                    for subscriber in &subscribers {
                        subscriptions.subscribe(tx, author, subscriber).await?;
                    }
                    Ok::<_, sqlx::Error>(())
                })
            })
            .await?;
        crate::confirmed_fixture_outcome(outcome, "performance follow creation")?;
    }
    Ok(())
}

async fn create_audiences(
    storage: &PerformanceSeedStorage,
    authors: &[(Username, UserId)],
) -> anyhow::Result<Vec<Vec<common::ids::AudienceId>>> {
    let mut all = Vec::with_capacity(authors.len());
    for (_, author) in authors {
        let names = (0..5)
            .map(|index| format!("performance-{index}").parse())
            .collect::<Result<Vec<_>, _>>()?;
        let audiences = Arc::clone(&storage.audiences);
        let author = *author;
        let outcome = storage
            .write_scope
            .run(move |tx| {
                Box::pin(async move {
                    let mut ids = Vec::with_capacity(names.len());
                    for name in &names {
                        ids.push(audiences.create_audience(tx, author, name).await?);
                    }
                    Ok::<_, storage::AudienceError>(ids)
                })
            })
            .await
            .context("creating audiences")?;
        let ids = crate::confirmed_fixture_outcome(outcome, "performance audience creation")?;
        all.push(ids);
    }
    Ok(all)
}

async fn populate_audience_memberships(
    storage: &PerformanceSeedStorage,
    authors: &[(Username, UserId)],
    audiences: &[Vec<common::ids::AudienceId>],
) -> anyhow::Result<()> {
    for ((_, author), audience_ids) in authors.iter().zip(audiences) {
        let members = storage
            .subscriptions
            .list_subscribers(*author)
            .await?
            .into_iter()
            .map(|subscription| subscription.subscription_id)
            .collect::<Vec<_>>();
        let audience_storage = Arc::clone(&storage.audiences);
        let ids = audience_ids.clone();
        let author = *author;
        let outcome = storage
            .write_scope
            .run(move |tx| {
                Box::pin(async move {
                    for audience in ids {
                        for member in &members {
                            audience_storage
                                .add_member(tx, author, audience, *member)
                                .await?;
                        }
                    }
                    Ok::<_, storage::AudienceError>(())
                })
            })
            .await
            .context("adding audience members")?;
        crate::confirmed_fixture_outcome(outcome, "performance audience membership creation")?;
    }
    Ok(())
}

async fn create_media(
    storage: &PerformanceSeedStorage,
    storage_path: &Path,
    authors: &[(Username, UserId)],
    clock: UtcInstant,
) -> anyhow::Result<Vec<Vec<MediaRef>>> {
    let mut all = Vec::with_capacity(authors.len());
    for (_, author) in authors {
        let mut author_media = Vec::with_capacity(MEDIA_BLOBS.len());
        for (index, bytes) in MEDIA_BLOBS.iter().enumerate() {
            let sha256 = ContentHash::from_digest(sha2::Sha256::digest(bytes).into());
            let filename = Filename::sanitized(&format!("performance-{index}.txt"))?;
            let media = MediaRef {
                source: MediaSource::Upload,
                sha256,
                filename,
            };
            let path = storage_path.join("media").join(media_path(
                &media.source,
                &media.sha256,
                &media.filename,
            ));
            publish_media_blob(&path, bytes)?;
            author_media.push(media);
        }
        let size_bytes = ByteSize::try_from(i64::try_from(MEDIA_BLOBS[0].len())?)?;
        let records = author_media
            .iter()
            .map(|media| MediaRecord {
                user_id: *author,
                sha256: media.sha256.clone(),
                filename: media.filename.clone(),
                source: media.source,
                content_type: detect_content_type(&media.filename),
                size_bytes,
                source_url: None,
                created_at: clock,
            })
            .collect::<Vec<_>>();
        let media_storage = Arc::clone(&storage.media);
        let outcome = storage
            .write_scope
            .run(move |tx| {
                Box::pin(async move {
                    for record in &records {
                        media_storage.create_media(tx, record).await?;
                    }
                    Ok::<_, storage::CreateMediaError>(())
                })
            })
            .await?;
        crate::confirmed_fixture_outcome(outcome, "performance Media creation")?;
        all.push(author_media);
    }
    Ok(all)
}

fn publish_media_blob(target: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .context("canonical Media path has no parent")?;
    fs::create_dir_all(parent)?;
    match fs::read(target) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => bail!(
            "existing canonical Media content differs at {}",
            target.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let temporary = target.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    match fs::hard_link(&temporary, target) {
        Ok(()) => fs::remove_file(temporary)?,
        // cov:ignore-start: deterministically forcing a competing hard-link publication requires an OS race
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(temporary)?;
            if fs::read(target)? != bytes {
                bail!(
                    "canonical Media content raced with different bytes at {}",
                    target.display()
                );
            }
        }
        // cov:ignore-stop
        Err(error) => return Err(error.into()), // cov:ignore: arbitrary hard-link OS failure requires filesystem fault injection
    }
    Ok(())
}

fn build_post_input(
    index: usize,
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    audiences: &[Vec<common::ids::AudienceId>],
    media: &[Vec<MediaRef>],
    clock: UtcInstant,
    backdated_count: usize,
) -> anyhow::Result<storage::CreatePostInput> {
    let index_u64 = u64::try_from(index)?;
    let author_index = index % authors.len();
    let body_bucket = bucket(index_u64, &plan.body_distribution);
    let format = match body_bucket.as_str() {
        value if value.starts_with("markdown") => PostFormat::Markdown,
        value if value.starts_with("html") => PostFormat::Html,
        value if value.starts_with("plain_text") => PostFormat::Org,
        _ => unreachable!("canonical body formats"),
    };
    let attachment_count = match bucket(index_u64, &plan.media_distribution).as_str() {
        "none" => 0,
        "one" => 1,
        "five" => 5,
        _ => unreachable!("canonical media distribution"),
    };
    let body = body_with_media(
        index,
        &body_bucket,
        format,
        &media[author_index][..attachment_count],
    )?; // cov:ignore: validated fixture plans always provide a compatible body bucket and Media slice
    let lifecycle = bucket(index_u64, &plan.lifecycle[..4]);
    let published_at = post_published_at(index, &lifecycle, backdated_count, clock)?;
    let audience_targets = match bucket(index_u64, &plan.audience_distribution).as_str() {
        "public_only" => vec![AudienceTarget::Public],
        "one_private" => vec![AudienceTarget::Named(audiences[author_index][0])],
        "five_private" => audiences[author_index]
            .iter()
            .copied()
            .map(AudienceTarget::Named)
            .collect(),
        _ => unreachable!("canonical audience distribution"),
    };
    let tag_count = match bucket(index_u64, &plan.tag_distribution).as_str() {
        "none" => 0,
        "two" => 2,
        "eight" => 8,
        _ => unreachable!("canonical tag distribution"),
    };
    let tags = (0..tag_count)
        .map(|tag| format!("perf-{index}-{tag}").parse::<TagLabel>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(render_post_input(RenderedPostContent {
        user_id: authors[author_index].1,
        title: Some(format!("Performance Post {index}").parse::<PostTitle>()?),
        slug: format!("performance-{index:06}").parse::<Slug>()?,
        body,
        format,
        published_at,
        summary: None,
        audiences: audience_targets,
        tags,
        idempotency_key: None,
        expectations: PostBookkeepingExpectation::default(),
    }))
}

fn post_published_at(
    index: usize,
    lifecycle: &str,
    backdated_count: usize,
    clock: UtcInstant,
) -> anyhow::Result<Option<UtcInstant>> {
    match lifecycle {
        "live" if index < backdated_count => {
            let hours = i64::try_from(index)?
                .checked_add(1)
                .context("backdated fixture clock offset overflows")?;
            Ok(Some(UtcInstant::from(
                clock
                    .value()
                    .checked_sub(hours.hours())
                    .context("fixed clock range")?,
            )))
        }
        "live" => Ok(Some(clock)),
        "scheduled" => Ok(Some(UtcInstant::from(
            clock
                .value()
                .checked_add(SCHEDULED_OFFSET_HOURS.hours())
                .context("fixed clock range")?,
        ))),
        _ => Ok(None),
    }
}

async fn create_posts(
    storage: &PerformanceSeedStorage,
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    audiences: &[Vec<common::ids::AudienceId>],
    media: &[Vec<MediaRef>],
    clock: UtcInstant,
    audit: &mut PerformanceSeedAudit,
) -> anyhow::Result<Vec<SeededPost>> {
    let post_count = usize::try_from(plan.posts)?;
    let backdated_count = usize::try_from(plan.lifecycle[4].count)?;
    let mut ids = Vec::with_capacity(post_count);
    for batch_start in (0..post_count).step_by(POST_BATCH_SIZE) {
        let batch_end = (batch_start + POST_BATCH_SIZE).min(post_count);
        let inputs = (batch_start..batch_end)
            .map(|index| {
                build_post_input(
                    index,
                    plan,
                    authors,
                    audiences,
                    media,
                    clock,
                    backdated_count,
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let posts = Arc::clone(&storage.posts);
        let outcome = storage
            .write_scope
            .run(move |tx| Box::pin(async move { posts.create_posts(tx, &inputs).await }))
            .await?;
        let batch_ids =
            crate::confirmed_fixture_outcome(outcome, "performance post batch creation")?;
        for (index, id) in (batch_start..batch_end).zip(batch_ids) {
            let lifecycle = bucket(index as u64, &plan.lifecycle[..4]);
            if lifecycle != "deleted" {
                audit.record_confirmed(format!("lifecycle.{lifecycle}"));
            }
            audit.record_confirmed(format!(
                "tags.{}",
                bucket(index as u64, &plan.tag_distribution)
            ));
            audit.record_confirmed(format!(
                "audiences.{}",
                bucket(index as u64, &plan.audience_distribution)
            ));
            audit.record_confirmed(format!(
                "body.{}",
                bucket(index as u64, &plan.body_distribution)
            ));
            audit.record_confirmed(format!(
                "media.{}",
                bucket(index as u64, &plan.media_distribution)
            ));
            if index < backdated_count {
                audit.record_confirmed("lifecycle.backdated_live");
            }
            ids.push(SeededPost { id, index });
        }
    }
    Ok(ids)
}

async fn apply_revisions(
    storage: &PerformanceSeedStorage,
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    posts: &[SeededPost],
    clock: UtcInstant,
    audit: &mut PerformanceSeedAudit,
) -> anyhow::Result<()> {
    for batch in posts.chunks(POST_BATCH_SIZE) {
        let targets = batch
            .iter()
            .map(|seeded| revision_target(plan, authors, *seeded))
            .collect::<Vec<_>>();
        let post_storage = Arc::clone(&storage.posts);
        let outcome = storage
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    for target in targets {
                        apply_post_revisions(&*post_storage, transaction, target, clock).await?;
                    }
                    Ok::<_, storage::UpdatePostError>(())
                })
            })
            .await?;
        crate::confirmed_fixture_outcome(outcome, "performance revision batch creation")?;
        record_confirmed_revisions(audit, plan, batch);
    }
    Ok(())
}

fn revision_target(
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    seeded: SeededPost,
) -> RevisionTarget {
    let deleted = bucket(seeded.index as u64, &plan.lifecycle[..4]) == "deleted";
    let revisions = revision_count(seeded.index as u64, plan);
    RevisionTarget {
        post_id: seeded.id,
        author: authors[seeded.index % authors.len()].1,
        revisions: revisions - u64::from(deleted),
        deleted,
    }
}

fn revision_count(index: u64, plan: &performance::DatasetPlan) -> u64 {
    match revision_bucket(index, plan).as_str() {
        "one" => 1,
        "five" => 5,
        "thirty_three" => 33,
        "base" => plan.revisions / plan.posts,
        "one_extra" => plan.revisions / plan.posts + 1,
        _ => unreachable!("deterministic revision distribution"),
    }
}

fn revision_bucket(index: u64, plan: &performance::DatasetPlan) -> String {
    let index = if *plan == canonical_plan(plan.profile) {
        revision_bucket_index(index, plan)
    } else {
        index
    };
    bucket(index, &plan.revision_distribution)
}

fn maximum_revision_count(plan: &performance::DatasetPlan) -> u64 {
    plan.revisions / plan.posts + u64::from(!plan.revisions.is_multiple_of(plan.posts))
}
fn revision_bucket_index(index: u64, plan: &performance::DatasetPlan) -> u64 {
    let author = index % plan.authors;
    let round = index / plan.authors;
    let posts_per_author = plan.posts / plan.authors;
    (plan.authors - 1 - author) * posts_per_author + round
}

async fn apply_post_revisions(
    posts: &dyn PostStorage,
    transaction: &mut storage::WriteTransaction,
    target: RevisionTarget,
    clock: UtcInstant,
) -> Result<(), storage::UpdatePostError> {
    let existing = posts
        .get_post_by_id(target.post_id, &ViewerIdentity::local(target.author))
        .await?
        .ok_or(storage::UpdatePostError::NotFound)?;
    let audiences = posts.get_post_audiences(target.post_id).await?;
    let tags = existing
        .tags
        .iter()
        .map(|tag| tag.tag_display.clone())
        .collect::<Vec<_>>();
    let publish = match existing.published_at {
        Some(at) => storage::PublishUpdate::Publish { at: Some(at) },
        None => storage::PublishUpdate::Unpublish,
    };
    let mut body = existing.body;
    for revision in 0..target.revisions {
        body = revised_body(body, revision)?;
        let rendered = host::render::with_media(&body, &existing.format);
        let input = storage::UpdatePostInput {
            title: existing.title.clone(),
            slug: existing.slug.clone(),
            body,
            format: existing.format,
            rendered,
            publish,
            summary: existing.summary.clone(),
            audiences: audiences.clone(),
            tags: Some(tags.clone()),
            request_clock: clock,
            expectations: PostBookkeepingExpectation::default(),
        };
        posts
            .update_post(transaction, target.post_id, target.author, &input)
            .await?;
        body = input.body;
    }
    if target.deleted {
        posts
            .soft_delete_post(transaction, target.post_id, target.author, clock)
            .await?;
    }
    Ok(())
}

fn revised_body(body: PostBody, revision: u64) -> Result<PostBody, storage::UpdatePostError> {
    let mut source = String::from(body);
    let marker_position = source.find('x').ok_or_else(|| {
        storage::UpdatePostError::Internal(sqlx::Error::Protocol(
            "performance body has no mutable marker".to_owned(),
        ))
    })?;
    let Ok(revision) = u8::try_from(revision % 26) else {
        unreachable!("modulo 26 always fits in u8")
    };
    let marker = char::from(b'a' + revision);
    let mut marker_buffer = [0; 4];
    source.replace_range(
        marker_position..=marker_position,
        marker.encode_utf8(&mut marker_buffer),
    );
    match source.parse() {
        Ok(body) => Ok(body),
        Err(_) => unreachable!("replacing one ASCII body byte preserves a valid PostBody"),
    }
}

fn record_confirmed_revisions(
    audit: &mut PerformanceSeedAudit,
    plan: &performance::DatasetPlan,
    posts: &[SeededPost],
) {
    for seeded in posts {
        if bucket(seeded.index as u64, &plan.lifecycle[..4]) == "deleted" {
            audit.record_confirmed("lifecycle.deleted");
        }
        let revision = revision_bucket(seeded.index as u64, plan);
        audit.record_confirmed(format!("revisions.{revision}"));
    }
}

async fn audit_persisted(
    storage: &PerformanceSeedStorage,
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    posts: &[SeededPost],
    clock: UtcInstant,
) -> anyhow::Result<BTreeMap<String, u64>> {
    let mut audit = PerformanceSeedAudit::default();
    for seeded in posts {
        let owner = authors[seeded.index % authors.len()].1;
        let summary = storage
            .posts
            .get_current_revision_summary(owner, seeded.id, clock)
            .await?
            .context("seeded post summary is missing")?;
        let history = collect_history(&storage.posts, owner, Some(seeded.id)).await?;
        let lifecycle = match summary.lifecycle {
            PostLifecycle::Published => "live",
            PostLifecycle::Draft => "draft",
            PostLifecycle::Scheduled => "scheduled",
            PostLifecycle::Deleted => "deleted",
        };
        audit.record_persisted(format!("lifecycle.{lifecycle}"));
        if summary.lifecycle == PostLifecycle::Published
            && summary
                .published_at
                .is_some_and(|published| published < clock)
        {
            audit.record_persisted("lifecycle.backdated_live");
        }
        let observation = observe_persisted_post(storage, owner, seeded.id, &history).await?;
        record_persisted_post_shape(&mut audit, plan, *seeded, &observation, history.len())?;
    }
    Ok(audit.persisted)
}

async fn observe_persisted_post(
    storage: &PerformanceSeedStorage,
    owner: UserId,
    post_id: PostId,
    history: &[storage::PostRevisionMetadata],
) -> anyhow::Result<PersistedPostObservation> {
    let current = storage
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::local(owner))
        .await?;
    if let Some(post) = current {
        return Ok(PersistedPostObservation {
            format: post.format,
            body_len: post.body.len(),
            tag_count: post.tags.len(),
            audiences: storage.posts.get_post_audiences(post_id).await?,
            media_count: host::render::extract_media_refs(post.rendered_html.as_ref()).len(),
        });
    } // cov:ignore: the covered owner-visible path returns before this defensive fallback boundary
    // cov:ignore-start: both storage backends retain owner-visible soft-deleted posts; this is a defensive backend fallback
    let revision = history
        .first()
        .context("deleted post has no final revision")?;
    let detail = storage
        .posts
        .get_post_revision_detail(owner, post_id, revision.revision_id)
        .await?
        .context("deleted post final revision detail is missing")?;
    let revision = detail.revision;
    Ok(PersistedPostObservation {
        format: revision.format,
        body_len: revision.body.len(),
        tag_count: revision.tags.len(),
        audiences: revision.audiences,
        media_count: revision.media.len(),
    })
    // cov:ignore-stop
}

fn record_persisted_post_shape(
    audit: &mut PerformanceSeedAudit,
    plan: &performance::DatasetPlan,
    seeded: SeededPost,
    observation: &PersistedPostObservation,
    history_len: usize,
) -> anyhow::Result<()> {
    let body_prefix = match observation.format {
        PostFormat::Markdown => "markdown",
        PostFormat::Html => "html",
        PostFormat::Org => "plain_text",
    };
    let body = plan
        .body_distribution
        .iter()
        .find(|item| {
            item.bucket.starts_with(body_prefix)
                && item.bucket.ends_with(&format!("_{}", observation.body_len))
        })
        .context("persisted body has no canonical format/length bucket")?;
    audit.record_persisted(format!("body.{}", body.bucket));
    audit.record_persisted(match observation.tag_count {
        0 => "tags.none",
        2 => "tags.two",
        8 => "tags.eight",
        _ => bail!(
            "persisted post has noncanonical tag count {}",
            observation.tag_count
        ),
    });
    audit.record_persisted(match observation.audiences.as_slice() {
        [AudienceTarget::Public] => "audiences.public_only",
        [AudienceTarget::Named(_)] => "audiences.one_private",
        values
            if values.len() == 5
                && values
                    .iter()
                    .all(|value| matches!(value, AudienceTarget::Named(_))) =>
        {
            "audiences.five_private"
        }
        _ => bail!("persisted post has noncanonical audiences"),
    });
    audit.record_persisted(match observation.media_count {
        0 => "media.none",
        1 => "media.one",
        5 => "media.five",
        _ => bail!(
            "persisted post has noncanonical media reference count {}",
            observation.media_count
        ),
    });
    if *plan != canonical_plan(plan.profile) {
        let expected = revision_count(seeded.index as u64, plan);
        if u64::try_from(history_len)? != expected {
            bail!("persisted post has {history_len} revisions; expected {expected} from the plan");
        }
        audit.record_persisted(format!(
            "revisions.{}",
            revision_bucket(seeded.index as u64, plan)
        ));
        return Ok(());
    }
    audit.record_persisted(match history_len {
        1 => "revisions.one",
        5 => "revisions.five",
        32 => "revisions.thirty_three",
        count => bail!("persisted post has noncanonical revision count {count}"),
    });
    Ok(())
}

async fn resolve_manifest(
    posts: &Arc<dyn PostStorage>,
    plan: &performance::DatasetPlan,
    authors: &[(Username, UserId)],
    post_ids: &[PostId],
    clock: UtcInstant,
) -> anyhow::Result<DatasetManifest> {
    let (username, owner) = &authors[0];
    let history_post_id = if *plan == canonical_plan(plan.profile) {
        post_ids.iter().enumerate().rev().find(|(index, _)| {
            *index % authors.len() == 0 && revision_bucket(*index as u64, plan) == "thirty_three"
        })
    } else {
        post_ids.iter().enumerate().find(|(index, _)| {
            *index % authors.len() == 0
                && revision_count(*index as u64, plan) == maximum_revision_count(plan)
        })
    }
    .map(|(_, id)| *id)
    .context("missing maximum-revision post")?;
    let revision = posts
        .list_post_revision_history(*owner, history_post_id, None, PageSize::default())
        .await?
        .context("history post missing")?
        .revisions
        .first()
        .context("maximum-revision post has no history")?
        .revision_id;
    let public = collect_timeline(posts, ViewerIdentity::Anonymous, clock).await?;
    let authenticated = collect_timeline(posts, ViewerIdentity::local(*owner), clock).await?;
    let app = collect_app_timeline(posts, username, *owner, clock).await?;
    let owner_history = collect_history(posts, *owner, None).await?;
    let post_history = collect_history(posts, *owner, Some(history_post_id)).await?;
    Ok(DatasetManifest {
        schema_version: performance::DATASET_SCHEMA_VERSION,
        plan: plan.clone(),
        subjects: WorkloadSubjects {
            username: username.to_string(),
            history_post_id: i64::from(history_post_id).cast_unsigned(),
            revision_id: i64::from(revision).cast_unsigned(),
            browser_initial_rows: BrowserInitialRows {
                home: public.len() as u64,
                app: app.len() as u64,
                global_history: owner_history.len() as u64,
                post_history: post_history.len() as u64,
            },
        },
        cursors: vec![
            timeline_cursor(Workload::PublicTimeline, &public)?,
            timeline_cursor(Workload::AuthenticatedTimeline, &authenticated)?,
            history_cursor(Workload::OwnerHistory, &owner_history)?,
            history_cursor(Workload::PostHistory, &post_history)?,
        ],
    })
}

async fn collect_timeline(
    posts: &Arc<dyn PostStorage>,
    viewer: ViewerIdentity,
    clock: UtcInstant,
) -> anyhow::Result<Vec<storage::PostRecord>> {
    let mut out = Vec::new();
    let limit = PageSize::default().fetch_limit();
    let mut cursor = None;
    loop {
        let page = match &cursor {
            Some(cursor) => PublishedPageRequest::after(cursor, limit),
            None => PublishedPageRequest::first(TimelineOrder::Newest, limit),
        };
        let rows = posts.list_published(page, &viewer, clock).await?;
        if rows.is_empty() {
            break;
        }
        cursor = Some(storage::to_post_cursor(
            rows.last().context("nonempty page")?,
            TimelineOrder::Newest,
        )?); // cov:ignore: persisted timeline records always contain cursor-compatible timestamps and ids
        out.extend(rows);
    }
    Ok(out)
}

async fn collect_app_timeline(
    posts: &Arc<dyn PostStorage>,
    username: &Username,
    owner: UserId,
    clock: UtcInstant,
) -> anyhow::Result<Vec<storage::PostRecord>> {
    let mut out = Vec::new();
    let limit = PageSize::default().fetch_limit();
    let mut cursor = None;
    loop {
        let page = match &cursor {
            Some(cursor) => PublishedPageRequest::after(cursor, limit),
            None => PublishedPageRequest::first(TimelineOrder::Newest, limit),
        };
        let rows = posts
            .list_published_by_user(username, page, &ViewerIdentity::local(owner), clock)
            .await?;
        if rows.is_empty() {
            break;
        }
        cursor = rows
            .last()
            .map(|record| storage::to_post_cursor(record, TimelineOrder::Newest))
            .transpose()?;
        out.extend(rows);
    }
    Ok(out)
}

async fn collect_history(
    posts: &Arc<dyn PostStorage>,
    owner: UserId,
    post: Option<PostId>,
) -> anyhow::Result<Vec<storage::PostRevisionMetadata>> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = match (post, cursor) {
            (Some(post), cursor) => posts
                .list_post_revision_history(owner, post, cursor, PageSize::default())
                .await?
                .context("history post missing")?,
            (None, cursor) => {
                posts
                    .list_owned_revision_history(owner, cursor, PageSize::default())
                    .await?
            }
        };
        if page.revisions.is_empty() {
            break;
        }
        cursor = Some(PostRevisionCursor {
            revision_id: page
                .revisions
                .last()
                .context("nonempty history page")?
                .revision_id,
        });
        out.extend(page.revisions);
    }
    Ok(out)
}

fn body_with_media(
    index: usize,
    body_bucket: &str,
    format: PostFormat,
    media: &[MediaRef],
) -> anyhow::Result<PostBody> {
    let (_, length) = body_bucket.rsplit_once('_').context("valid body bucket")?;
    let length = length.parse::<usize>()?;
    let links = media
        .iter()
        .map(|media| {
            let url = media_url(&media.source, &media.sha256, &media.filename);
            match format {
                PostFormat::Markdown => format!("![performance attachment]({url})"),
                PostFormat::Html => format!("<img src=\"{url}\" alt=\"performance attachment\">"),
                PostFormat::Org => format!("[[{url}][performance attachment]]"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prefix = match format {
        PostFormat::Markdown => format!("# {index:06}\n\n{links}\n\n"),
        PostFormat::Html => format!("<p>{index:06}</p>{links}"),
        PostFormat::Org => format!("* {index:06}\n\n{links}\n\n"),
    };
    let fill = length
        .checked_sub(prefix.len())
        .context("media markup exceeds body bucket")?;
    format!("{prefix}{}", "x".repeat(fill))
        .parse()
        .map_err(Into::into)
}

fn timeline_cursor(workload: Workload, rows: &[storage::PostRecord]) -> anyhow::Result<Cursor> {
    let rank = rank(rows.len())?;
    let row = rows
        .get(rank - 1)
        .context("timeline has no matching rows")?;
    Ok(Cursor {
        workload,
        target_percent: 80,
        matching_result_count: rows.len() as u64,
        resolved_rank: rank as u64,
        cursor: PersistedCursor::Timeline(TimelineCursor {
            created_at_us: row
                .published_at
                .context("timeline row has no publication time")?
                .value()
                .as_microsecond(),
            post_id: i64::from(row.post_id).cast_unsigned(),
        }),
    })
}
fn history_cursor(
    workload: Workload,
    rows: &[storage::PostRevisionMetadata],
) -> anyhow::Result<Cursor> {
    let rank = rank(rows.len())?;
    let row = rows.get(rank - 1).context("history has no matching rows")?;
    Ok(Cursor {
        workload,
        target_percent: 80,
        matching_result_count: rows.len() as u64,
        resolved_rank: rank as u64,
        cursor: PersistedCursor::History(HistoryCursor {
            revision_id: i64::from(row.revision_id).cast_unsigned(),
        }),
    })
}
fn rank(count: usize) -> anyhow::Result<usize> {
    if count == 0 {
        bail!("empty matching result set")
    }
    Ok((count * 80).div_ceil(100))
}
fn bucket(index: u64, distribution: &[performance::Distribution]) -> String {
    let mut offset = 0;
    for item in distribution {
        offset += item.count;
        if index < offset {
            return item.bucket.clone();
        }
    }
    unreachable!("canonical distribution covers all posts")
}

/// Validate then atomically replace the sole manifest file below `output`.
///
/// # Errors
///
/// Returns an error when the manifest is invalid or its temporary or destination file cannot be
/// written or replaced.
pub fn write_manifest(output: &Path, manifest: &DatasetManifest) -> anyhow::Result<PathBuf> {
    validate_manifest(manifest)?;
    fs::create_dir_all(output)?;
    let destination = output.join(performance::DATASET_MANIFEST_FILENAME);
    let temporary = output.join(format!(".{}.tmp", performance::DATASET_MANIFEST_FILENAME));
    let encoded = serde_json::to_vec(manifest)?;
    fs::write(&temporary, encoded)?;
    fs::rename(&temporary, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> PersistedPostObservation {
        PersistedPostObservation {
            format: PostFormat::Markdown,
            body_len: 256,
            tag_count: 0,
            audiences: vec![AudienceTarget::Public],
            media_count: 0,
        }
    }

    fn seeded_post() -> SeededPost {
        SeededPost {
            id: PostId::from(1_i64),
            index: 0,
        }
    }

    #[test]
    fn audit_rejects_different_confirmed_and_persisted_counts() {
        let mut audit = PerformanceSeedAudit::default();
        audit.record_confirmed("lifecycle.live");

        let error = audit.verify().expect_err("different audits must fail");
        assert!(error.to_string().contains("audits differ"), "{error}");
    }

    #[test]
    fn media_publication_is_idempotent_and_rejects_conflicting_content() {
        let directory = tempfile::tempdir().expect("Media directory");
        let target = directory.path().join("blob");

        publish_media_blob(&target, b"expected").expect("publish new content");
        publish_media_blob(&target, b"expected").expect("accept identical content");
        let error =
            publish_media_blob(&target, b"different").expect_err("reject conflicting content");
        assert!(
            error.to_string().contains("content differs"),
            "unexpected conflict error: {error}"
        );

        let error = publish_media_blob(directory.path(), b"bytes")
            .expect_err("a directory is not readable Media content");
        assert!(
            error.to_string().contains("Is a directory"),
            "unexpected read error: {error}"
        );
    }

    #[test]
    fn revised_body_rejects_content_without_the_fixture_marker() {
        let body: PostBody = "no mutable marker".parse().expect("valid body");
        let error = revised_body(body, 1).expect_err("missing marker must fail");
        assert!(
            error.to_string().contains("no mutable marker"),
            "unexpected revision error: {error}"
        );
    }

    #[test]
    fn body_media_markup_covers_each_supported_post_format() {
        let bytes = MEDIA_BLOBS[0];
        let media = MediaRef {
            source: MediaSource::Upload,
            sha256: ContentHash::from_digest(sha2::Sha256::digest(bytes).into()),
            filename: Filename::sanitized("performance.txt").expect("valid filename"),
        };

        for (format, bucket, marker) in [
            (
                PostFormat::Markdown,
                "markdown_256",
                "![performance attachment]",
            ),
            (PostFormat::Html, "html_256", "<img src="),
            (PostFormat::Org, "plain_text_256", "[[/media/"),
        ] {
            let body = body_with_media(0, bucket, format, std::slice::from_ref(&media))
                .expect("fixture body");
            assert!(
                String::from(body).contains(marker),
                "{format:?} uses its canonical Media markup"
            );
        }
    }

    #[test]
    fn persisted_shape_validation_rejects_noncanonical_observations() {
        let canonical = canonical_plan(DatasetProfile::Small);
        for (observation, expected) in [
            (
                PersistedPostObservation {
                    tag_count: 3,
                    ..observation()
                },
                "noncanonical tag count",
            ),
            (
                PersistedPostObservation {
                    audiences: vec![AudienceTarget::Public, AudienceTarget::Public],
                    ..observation()
                },
                "noncanonical audiences",
            ),
            (
                PersistedPostObservation {
                    media_count: 2,
                    ..observation()
                },
                "noncanonical media reference count",
            ),
        ] {
            let error = record_persisted_post_shape(
                &mut PerformanceSeedAudit::default(),
                &canonical,
                seeded_post(),
                &observation,
                1,
            )
            .expect_err("noncanonical observation must fail");
            assert!(
                error.to_string().contains(expected),
                "unexpected shape error: {error}"
            );
        }

        let error = record_persisted_post_shape(
            &mut PerformanceSeedAudit::default(),
            &canonical,
            seeded_post(),
            &observation(),
            2,
        )
        .expect_err("canonical revision count must match");
        assert!(
            error.to_string().contains("noncanonical revision count"),
            "unexpected history error: {error}"
        );

        let custom = plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(120),
                authors: Some(12),
                revisions: Some(777),
            },
        )
        .expect("valid custom plan");
        let error = record_persisted_post_shape(
            &mut PerformanceSeedAudit::default(),
            &custom,
            seeded_post(),
            &observation(),
            1,
        )
        .expect_err("custom revision count must match");
        assert!(
            error.to_string().contains("expected"),
            "unexpected custom history error: {error}"
        );
    }

    #[test]
    fn cursor_rank_rejects_an_empty_result_set() {
        let error = rank(0).expect_err("empty rows cannot resolve a cursor");
        assert_eq!(error.to_string(), "empty matching result set");
    }
}
