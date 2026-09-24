//! Persisted title/slug search projection maintenance.

use sqlx::Pool;

use crate::posts::store::PostDialect;
use common::ids::PostId;
use common::pagination::RowLimit;
use common::post_search::post_search_projection;
use common::post_title::PostTitle;
use common::slug::Slug;

const BACKFILL_BATCH_SIZE: i64 = 100;

/// Monotonic concurrency token for meaningful Post mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub struct PostMutationVersion(i64);

impl PostMutationVersion {
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    #[must_use]
    pub const fn value(self) -> i64 {
        self.0
    }

    /// Reconstitutes a wire concurrency token, rejecting impossible versions.
    #[must_use]
    pub const fn from_value(value: i64) -> Option<Self> {
        if value >= 1 { Some(Self(value)) } else { None }
    }
}

/// Normalized title/slug bytes persisted solely for storage-side matching.
#[derive(Clone, Debug, macros::SqlxBridge)]
pub(crate) struct StoredPostSearchText(String);

impl StoredPostSearchText {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for StoredPostSearchText {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// One derived value paired with the mutation version observed before derivation.
pub struct PostSearchBackfillCandidate {
    pub(crate) post_id: PostId,
    pub(crate) mutation_version: PostMutationVersion,
    pub(crate) search_text: StoredPostSearchText,
}

/// Completes nullable legacy search projections in bounded batches.
///
/// Each candidate carries its observed mutation version, so a concurrent content
/// update wins and a later startup derives that Post again from its newer state.
///
/// # Errors
///
/// Returns a storage error when a candidate read or batched write fails.
pub async fn backfill_post_search_projections<DB>(pool: &Pool<DB>) -> sqlx::Result<()>
where
    DB: PostDialect,
{
    let mut cursor = None;
    loop {
        let rows = DB::list_post_search_backfill_candidates(
            pool,
            cursor,
            RowLimit::at_most(BACKFILL_BATCH_SIZE),
        )
        .await?;
        let Some((last_post_id, ..)) = rows.last() else {
            return Ok(());
        };
        cursor = Some(*last_post_id);
        let candidates = rows
            .into_iter()
            .map(
                |(post_id, title, slug, mutation_version): (
                    PostId,
                    Option<PostTitle>,
                    Slug,
                    PostMutationVersion,
                )| PostSearchBackfillCandidate {
                    post_id,
                    mutation_version,
                    search_text: post_search_projection(title.as_ref(), &slug).into(),
                },
            )
            .collect::<Vec<_>>();
        DB::apply_post_search_backfill(pool, &candidates).await?;
    }
}
