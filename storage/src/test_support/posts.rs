//! Post fixture builders and batch seed helpers. It owns direct storage-layer inputs
//! and production-rendered seed records; service-path fixtures live in [`super::post_service`].

#[cfg(test)]
use super::TestEnv;
use super::{confirmed_for, fixture_media_content_locks};
use crate::AppState;
use crate::posts::{
    errors::CreatePostError,
    models::{
        CreatePostInput, PostBookkeepingExpectation, PostFormat, PostRecord, PublishUpdate,
        UpdatePostInput,
    },
};
#[cfg(test)]
use crate::sql::QueryStorageExt;

use common::ids::{PostId, UserId};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::RenderedHtml;
use common::slug::Slug;
use common::tag::TagLabel;
use common::test_support::{parse_post_body, parse_post_title, parse_slug, parse_tag_label};
use common::time::UtcInstant;
use common::visibility::AudienceTarget;
use host::render::with_media;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// `published == true` sets `published_at = now` so list/timeline endpoints
/// return them; `false` leaves them as drafts. Returns ids in creation order.
///
/// # Panics
///
/// If a slug fails to parse or a post fails to persist.
pub async fn seed_posts(
    state: &Arc<AppState>,
    user_id: UserId,
    count: usize,
    published: bool,
) -> Vec<PostId> {
    let inputs: Vec<_> = (0..count)
        .map(|i| {
            crate::seed_post_input(
                user_id,
                parse_slug(&format!("seed-{i}")),
                parse_post_body(&format!("# Post {i}\n\nbody")),
                published,
            )
        })
        .collect();
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.create_posts(transaction, &inputs).await })
        })
        .await
        .expect("seed posts should be created");
    confirmed_for(outcome, "seed posts")
}

/// Creates supplied storage-layer inputs through one state-owned write capability.
///
/// # Panics
///
/// Fixture writes require a confirmed commit.
pub async fn create_posts_confirmed(
    state: &Arc<AppState>,
    inputs: Vec<CreatePostInput>,
) -> Vec<PostId> {
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.create_posts(transaction, &inputs).await })
        })
        .await
        .expect("create post fixture should succeed");
    confirmed_for(outcome, "create post fixture")
}

/// A single seeded post, built the real [`perform_post_creation`](crate::perform_post_creation)
/// way — the same service-layer path production uses (renders the body, generates a
/// unique slug via collision-retry, re-reads the row). Aggressively defaulted: a
/// **published, public, Markdown** post with a fixed non-empty body, so the
/// overwhelming majority of call sites are the bare `SeedPost::new(user_id).seed(&state)`
/// and a setter appears only where a test asserts on (or requires) that field — the
/// [`SeedUser`](super::SeedUser) discipline.
///
/// Distinct from [`seed_posts`] (batch, generic `seed-{i}` posts) and from the
/// `create_post`-layer literals the storage-contract tests hand-roll (#656): those seed
/// *below* `perform_post_creation` and control `rendered_html`/slug explicitly, which
/// this builder deliberately does not.
///
/// Repeated bare seeds get **distinct** slugs: a title-less post derives its slug from
/// the fixed body, and `perform_post_creation`'s collision-suffix retry disambiguates
/// (`seeded-post-body`, `seeded-post-body-2`, …).
pub struct SeedPost {
    user_id: UserId,
    title: Option<PostTitle>,
    body: PostBody,
    audiences: Vec<AudienceTarget>,
}

impl SeedPost {
    /// A published, public, Markdown post owned by `user_id`, with a fixed non-empty
    /// body and no explicit title. Deviate from a default only where a test requires
    /// it. Only the three fields real call sites vary — title, body, audiences — are
    /// settable; the rest (Markdown, published-now, no slug/summary/idempotency) are
    /// fixed defaults, mirroring how `SeedUser` exposes only the setters its callers use.
    #[must_use]
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            title: None,
            body: parse_post_body("Seeded post body"),
            audiences: vec![AudienceTarget::Public],
        }
    }

    /// Set an explicit title — for the permalink/listing tests that assert on it.
    #[must_use]
    pub fn title(mut self, title: PostTitle) -> Self {
        self.title = Some(title);
        self
    }

    /// Override the default body.
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Replace the default `[Public]` audience targeting.
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Persist via [`perform_post_creation`](crate::perform_post_creation)
    /// (`max_attempts = 100`) and return the re-read [`PostRecord`] (carries `post_id`
    /// and `slug`).
    ///
    /// # Panics
    ///
    /// If the post cannot be created — happy-path setup only, like [`SeedUser::seed`](super::SeedUser::seed).
    pub async fn seed(self, state: &Arc<AppState>) -> PostRecord {
        let outcome = crate::perform_post_creation(
            &state.write_scope,
            &fixture_media_content_locks(),
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            crate::PostCreation {
                user_id: self.user_id,
                body: self.body,
                title: self.title.as_ref(),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: Some(UtcInstant::now()),
                max_attempts: 100,
                summary: None,
                audiences: self.audiences,
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .expect("seed post should be created");
        confirmed_for(outcome, "seed post")
    }
}

/// A post seeded by [`SeedRawPost`] — its id plus the values a test reads back instead of
/// hardcoding a literal (mirrors [`SeededUser`](super::SeededUser)). `body` is never read back, so it is not
/// carried here; `rendered_html` is the resolved `render(body)` (one page-render assertion
/// site embeds it); `published_at` is `None` for a `.draft()`.
#[derive(Debug)]
pub struct SeededPost {
    pub post_id: PostId,
    pub slug: Slug,
    pub title: PostTitle,
    pub published_at: Option<UtcInstant>,
    pub rendered_html: RenderedHtml,
}

/// Monotonic sequence behind [`SeedRawPost`]'s autogenerated slug + title. Private and
/// per-process; correctness rests on the same fresh-DB-per-test invariant [`SeedUser`](super::SeedUser)
/// documents (nextest runs process-per-test, so one counter serves one DB and the
/// `(user, slug, day)` uniqueness never trips on an accidental collision).
static RAW_POST_SEQ: AtomicU64 = AtomicU64::new(0);

/// Builder that seeds a post **directly through the `create_post` storage layer** — no
/// slug-retry, no service-layer massaging — a sibling to the service-layer post seeder
/// and distinct from the batch [`seed_posts`]. It defaults every field (autogenerated
/// unique slug `post-{n}` + title `"Post {n}"`, a fixed non-empty Markdown body,
/// `render(body)` HTML, published-now, Public); a call site overrides only what it varies.
///
/// `.seed`/`.create` return a [`SeededPost`] so a test reads back the autogenerated
/// slug/title (etc.) rather than owning a literal; `.seed` `expect()`s success like
/// [`SeedUser::seed`](super::SeedUser::seed), while `.create` hands back the `Result` for the conflict/FK tests
/// that assert the `Err`. `.build` yields the raw [`CreatePostInput`] for the batch tests.
///
/// There is deliberately **no** `.title`/`.idempotency_key`/`.rendered_html` setter: no
/// adopting site chooses a title, sets an idempotency key, or supplies rendered HTML — the
/// builder renders `body` with the production [`render`], so the HTML is always derived.
pub struct SeedRawPost {
    user_id: UserId,
    slug: Option<Slug>,
    body: PostBody,
    format: PostFormat,
    published_at: Option<UtcInstant>,
    summary: Option<PostSummary>,
    audiences: Vec<AudienceTarget>,
    tags: Vec<TagLabel>,
}

impl SeedRawPost {
    /// A published, Public, Markdown post owned by `user_id`, with an autogenerated unique
    /// slug + title and a fixed non-empty body.
    #[must_use]
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            slug: None,
            body: parse_post_body("seed body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
        }
    }

    /// Force a specific slug — for conflict *sameness* (`.slug(other.slug.as_ref())`) or a
    /// slug a test lists / looks up by.
    #[must_use]
    pub fn slug(mut self, slug: impl AsRef<str>) -> Self {
        self.slug = Some(parse_slug(slug.as_ref()));
        self
    }

    /// Override the body (e.g. embed a media URL). The rendered HTML re-derives from it.
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Override the markup format (the rendered HTML re-derives accordingly).
    #[must_use]
    pub fn format(mut self, format: PostFormat) -> Self {
        self.format = format;
        self
    }

    /// Attach a summary/excerpt.
    #[must_use]
    pub fn summary(mut self, summary: PostSummary) -> Self {
        self.summary = Some(summary);
        self
    }

    /// Replace the audience targeting (default `[Public]`).
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Seed as a draft (`published_at = None`).
    #[must_use]
    pub fn draft(mut self) -> Self {
        self.published_at = None;
        self
    }

    /// Seed with an exact publication instant (scheduled / backdated / go-live-window).
    #[must_use]
    pub fn published_at(mut self, at: UtcInstant) -> Self {
        self.published_at = Some(at);
        self
    }

    /// Tags attached atomically by the creation transaction.
    #[must_use]
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.tags = tags
            .into_iter()
            .map(|tag| parse_tag_label(tag.as_ref()))
            .collect();
        self
    }

    /// Resolve the autogenerated slug/title and rendered HTML into a [`CreatePostInput`]
    /// without writing.
    #[must_use]
    pub fn build(self) -> CreatePostInput {
        self.into_input()
    }

    fn into_input(self) -> CreatePostInput {
        let n = RAW_POST_SEQ.fetch_add(1, Ordering::Relaxed);
        let slug = self
            .slug
            .unwrap_or_else(|| parse_slug(&format!("post-{n}")));
        let title = parse_post_title(&format!("Post {n}"));
        let rendered = with_media(&self.body, &self.format);
        CreatePostInput {
            user_id: self.user_id,
            title: Some(title),
            slug,
            body: self.body,
            format: self.format,
            rendered,
            published_at: self.published_at,
            summary: self.summary,
            audiences: self.audiences,
            tags: self.tags,
            expectations: PostBookkeepingExpectation::default(),
            idempotency_key: None,
        }
    }

    /// Write via `create_post` and return the [`SeededPost`].
    ///
    /// # Errors
    ///
    /// Propagates the [`CreatePostError`] from `create_post`.
    ///
    /// # Panics
    ///
    /// Panics only if `SeedRawPost`'s invariant that it always generates a title
    /// is violated.
    pub async fn create(self, state: &Arc<AppState>) -> Result<SeededPost, CreatePostError> {
        let input = self.into_input();
        let slug = input.slug.clone();
        let title = input
            .title
            .clone()
            .expect("SeedRawPost always autogenerates a title");
        let published_at = input.published_at;
        let rendered_html = input.rendered.clone().into_html();
        let posts = Arc::clone(&state.posts);
        let outcome = state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .create_post(transaction, &input, UtcInstant::now())
                        .await
                })
            })
            .await
            .map_err(|error| match error {
                crate::WriteScopeError::Operation(error) => error,
                crate::WriteScopeError::Begin(error) => CreatePostError::Internal(error),
            })?;
        let post_id = confirmed_for(outcome, "seed raw post").record.post_id;
        Ok(SeededPost {
            post_id,
            slug,
            title,
            published_at,
            rendered_html,
        })
    }

    /// Happy-path seed: `create` + `expect`, like [`SeedUser::seed`](super::SeedUser::seed).
    ///
    /// # Panics
    ///
    /// If the post cannot be created.
    pub async fn seed(self, state: &Arc<AppState>) -> SeededPost {
        self.create(state)
            .await
            .expect("seed raw post should be created")
    }
}

/// Builder for an [`UpdatePostInput`] — the edit-side sibling of [`SeedRawPost`], with the
/// same defaults-plus-overrides shape. An update test typically varies one or two fields;
/// this builder defaults the rest so a test overrides only what it means.
///
/// Defaults: title `"Updated Title"`, body `"updated body"`, Markdown, no summary, `[Public]`,
/// and [`PublishUpdate::Publish`] without an explicit timestamp, which keeps an existing
/// publication timestamp or stamps `now` for a previously-unpublished Post. A test that
/// unpublishes says so with [`unpublish`][Self::unpublish]. The slug is the one required
/// argument because an update's slug is what collides (or does not) with a sibling Post, so
/// no default could be right.
///
/// `rendered` has no setter: [`build`][Self::build] derives it from `body`/`format` with the
/// production [`host::render::with_media`], exactly as `SeedRawPost` does, so no call
/// site re-spells the render and no input can carry a reference set that disagrees with its HTML
/// (#711).
///
/// `Clone` is load-bearing: the audience tests vary one field off a shared base via
/// `..base.clone()` struct-update spreads.
#[derive(Clone)]
pub struct UpdateRawPost {
    title: Option<PostTitle>,
    slug: Slug,
    body: PostBody,
    format: PostFormat,
    publish: PublishUpdate,
    summary: Option<PostSummary>,
    audiences: Vec<AudienceTarget>,
    tags: Vec<TagLabel>,
    request_clock: UtcInstant,
}

impl UpdateRawPost {
    /// A titled, Public, Markdown edit at `slug` that leaves publication alone.
    #[must_use]
    pub fn new(slug: impl AsRef<str>) -> Self {
        Self {
            title: Some(parse_post_title("Updated Title")),
            slug: parse_slug(slug.as_ref()),
            body: parse_post_body("updated body"),
            format: PostFormat::Markdown,
            publish: PublishUpdate::Publish { at: None },
            summary: None,
            request_clock: UtcInstant::now(),
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
        }
    }

    /// Override the title a test reads back.
    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(parse_post_title(title));
        self
    }

    /// Override the body (the rendered HTML and its media references re-derive from it).
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Override the markup format (the rendered HTML re-derives accordingly).
    #[must_use]
    pub fn format(mut self, format: PostFormat) -> Self {
        self.format = format;
        self
    }

    /// Clear `published_at` back to NULL (draft / unschedule).
    #[must_use]
    pub fn unpublish(mut self) -> Self {
        self.publish = PublishUpdate::Unpublish;
        self
    }

    /// Set — or, with `None`, clear — the summary/excerpt. Takes `impl Into<Option<_>>` so a
    /// test that only ever sets one reads like [`SeedRawPost::summary`], while the
    /// set-then-clear test passes its `Option` straight through.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<Option<PostSummary>>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Replace the audience targeting (default `[Public]`).
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Set the request clock for a deterministic update timestamp.
    #[must_use]
    pub fn request_clock(mut self, request_clock: UtcInstant) -> Self {
        self.request_clock = request_clock;
        self
    }

    /// Replace tags within the same update transaction.
    #[must_use]
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.tags = tags
            .into_iter()
            .map(|tag| parse_tag_label(tag.as_ref()))
            .collect();
        self
    }

    /// Resolve into the [`UpdatePostInput`] to hand `update_post`, rendering `body` here.
    #[must_use]
    pub fn build(self) -> UpdatePostInput {
        let rendered = with_media(&self.body, &self.format);
        UpdatePostInput {
            title: self.title,
            slug: self.slug,
            body: self.body,
            format: self.format,
            rendered,
            publish: self.publish,
            summary: self.summary,
            audiences: self.audiences,
            tags: Some(self.tags),
            request_clock: self.request_clock,
            expectations: PostBookkeepingExpectation::default(),
        }
    }
}
#[cfg(test)]
#[derive(macros::SqlxBridge)]
struct PostRevisionCount(i64);

#[cfg(test)]
pub(crate) async fn count_post_revisions(
    env: &TestEnv,
    post_id: PostId,
) -> Result<i64, sqlx::Error> {
    crate::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query_scalar::<_, PostRevisionCount>(
            "SELECT COUNT(*) FROM post_revisions WHERE post_id = $1",
        )
        .bind_storage(post_id)
        .fetch_one(pool)
        .await
        .map(|count| count.0)
    })
}

#[cfg(test)]
mod tests {
    use super::{SeedPost, SeedRawPost};
    use crate::posts::{errors::CreatePostError, models::PostFormat};
    use crate::test_support::{Backend, SeedUser, backends};

    use common::post_summary::PostSummary;
    use common::test_support::{parse_post_body, parse_post_title, parse_row_limit};
    use common::time::UtcInstant;
    use common::visibility::{AudienceTarget, ViewerIdentity};
    use host::render::render;
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_builder_defaults_create_published_public_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        let post = SeedPost::new(user.user_id).seed(state).await;
        assert!(
            post.published_at.is_some(),
            "default post should be published"
        );
        assert!(!post.slug.as_ref().is_empty(), "post should have a slug");
        assert!(!post.body.as_ref().is_empty(), "post should have a body");
        assert_eq!(post.format, PostFormat::Markdown);
        let audiences = state.posts.get_post_audiences(post.post_id).await.unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Public]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_builder_setters_apply(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        let post = SeedPost::new(user.user_id)
            .title(parse_post_title("Custom Title"))
            .body(parse_post_body("Custom body text"))
            .audiences(vec![AudienceTarget::Public])
            .seed(state)
            .await;
        assert_eq!(post.title.as_ref().map(AsRef::as_ref), Some("Custom Title"));
        assert!(post.body.as_ref().contains("Custom body text"));
        let audiences = state.posts.get_post_audiences(post.post_id).await.unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Public]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_bare_repeated_seeds_get_distinct_slugs(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        let a = SeedPost::new(user.user_id).seed(state).await;
        let b = SeedPost::new(user.user_id).seed(state).await;
        assert_ne!(a.slug, b.slug, "bare seeds should get distinct slugs");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_defaults_create_a_published_public_markdown_post(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author).seed(state).await;
        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert_eq!(record.slug, post.slug);
        assert_eq!(record.title, Some(post.title));
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.published_at.is_some(), "default is published");
        assert_eq!(record.rendered_html, post.rendered_html);
        assert_eq!(
            record.rendered_html,
            render(&record.body, &record.format),
            "default rendered_html equals render(body)"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_autogenerates_distinct_slugs_and_titles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let a = SeedRawPost::new(author).seed(state).await;
        let b = SeedRawPost::new(author).seed(state).await;
        assert_ne!(a.slug, b.slug, "each seed gets a fresh slug");
        assert_ne!(a.title, b.title, "each seed gets a fresh title");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_overrides_apply(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author)
            .draft()
            .format(PostFormat::Org)
            .summary(PostSummary::from_title(&parse_post_title("excerpt")))
            .tags(["rust"])
            .seed(state)
            .await;
        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert!(record.published_at.is_none(), "draft override applies");
        assert_eq!(record.format, PostFormat::Org);
        assert!(record.summary.is_some(), "summary override applies");
        assert_eq!(record.tags.len(), 1, "tag applied after insert");
        let targeted = SeedRawPost::new(author)
            .audiences(vec![AudienceTarget::Subscribers])
            .seed(state)
            .await;
        let audiences = state
            .posts
            .get_post_audiences(targeted.post_id)
            .await
            .unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Subscribers]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_create_surfaces_slug_conflict(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let first = SeedRawPost::new(author).seed(state).await;
        let err = SeedRawPost::new(author)
            .slug(first.slug.as_ref())
            .published_at(first.published_at.expect("default is published"))
            .create(state)
            .await
            .unwrap_err();
        assert!(matches!(err, CreatePostError::SlugConflict));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_body_override_is_persisted_and_rendered(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author)
            .body(parse_post_body("custom body"))
            .seed(state)
            .await;
        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert!(
            record.body.contains("custom body"),
            "body override persisted"
        );
        assert!(
            post.rendered_html.as_ref().contains("custom body"),
            "rendered HTML derives from the overridden body"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_build_yields_a_distinct_input_without_writing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await;
        let a = SeedRawPost::new(author.user_id).build();
        let b = SeedRawPost::new(author.user_id).build();
        assert!(a.title.is_some(), "build autogenerates a title");
        assert_ne!(a.slug, b.slug, "each build autogenerates a distinct slug");
        let published = state
            .posts
            .list_published_by_user(
                &author.username,
                None,
                parse_row_limit("50"),
                &ViewerIdentity::Anonymous,
                UtcInstant::now(),
            )
            .await
            .unwrap();
        assert!(published.is_empty(), "build() does not persist");
    }
}
