//! Tag persistence records, reconciliation SQL, and write planning.

use std::collections::HashSet;

use crate::sql::QueryStorageExt;
use sqlx::{Database, Decode, Encode, Executor, Result, Row, Type};

use common::ids::{PostId, TagId};
use common::tag::{Tag, TagLabel};

/// A tag record returned by [`super::store::PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct TagRecord {
    pub tag_id: TagId,
    pub tag_slug: Tag,
}

impl<'r, R> sqlx::FromRow<'r, R> for TagRecord
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    TagId: Decode<'r, R::Database> + Type<R::Database>,
    Tag: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        let tag_id = row.try_get::<TagId, _>("tag_id")?;
        let tag_slug = row.try_get::<Tag, _>("tag_slug")?;

        Ok(Self { tag_id, tag_slug })
    }
}

/// A post-tag association returned by [`super::store::PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct PostTag {
    pub post_id: PostId,
    pub tag_id: TagId,
    pub tag_slug: Tag,
    /// The original case-sensitive display name of the tag.
    pub tag_display: TagLabel,
}

impl<'r, R> sqlx::FromRow<'r, R> for PostTag
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    PostId: Decode<'r, R::Database> + Type<R::Database>,
    TagId: Decode<'r, R::Database> + Type<R::Database>,
    Tag: Decode<'r, R::Database> + Type<R::Database>,
    TagLabel: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        let post_id = row.try_get::<PostId, _>("post_id")?;
        let tag_id = row.try_get::<TagId, _>("tag_id")?;
        let tag_slug = row.try_get::<Tag, _>("tag_slug")?;
        let tag_display = row.try_get::<TagLabel, _>("tag_display")?;

        Ok(Self {
            post_id,
            tag_id,
            tag_slug,
            tag_display,
        })
    }
}

/// The escaped `LIKE` pattern for a normalized tag-slug prefix lookup.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct TagSlugPrefixPattern(String);

impl TagSlugPrefixPattern {
    pub(crate) fn from_normalized_prefix(prefix: &str) -> Self {
        Self(format!("{prefix}%"))
    }
}

pub(crate) const TAG_EXISTS_SQL: &str = "SELECT EXISTS(SELECT 1 FROM tags WHERE tag_slug = $1)";

/// The post's existing tags, read inside `set_post_tags`' transaction. The SQL is
/// identical on both dialects, so it is shared here rather than duplicated per
/// ADR-0019; only the surrounding transaction shape diverges. `ORDER BY` is not
/// needed for the diff (which is set-based) but keeps the read deterministic,
/// matching [`super::models::PostRecord::tags`] (#772).
pub(crate) const SELECT_POST_TAGS: &str = "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
     FROM post_tags pt
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE pt.post_id = $1
     ORDER BY t.tag_slug";

/// Get-or-create a tag by slug, returning its id in **one** statement.
///
/// The no-op `DO UPDATE` is load-bearing: `DO NOTHING` emits no row for
/// `RETURNING` on the conflict path, which would force a second `SELECT` and
/// open a window in which a concurrently deleted tag yields `RowNotFound`
/// (#883). Rewriting `tag_slug` to the value it already holds makes the id come
/// back on both the insert and the conflict path. #343 landed the same shape
/// for `subscriptions`; both dialects run it.
///
/// Shared rather than per-dialect: `SQLite` accepts `$n` placeholders and
/// `ON CONFLICT … DO UPDATE … RETURNING`.
///
/// **Takes a row lock on the tag until commit**, which is why
/// [`post_tag_diff`] hands additions back in slug order.
///
/// Bind order: `tag_slug`.
pub(crate) const UPSERT_TAG_RETURNING_ID: &str = "INSERT INTO tags (tag_slug) VALUES ($1)
     ON CONFLICT (tag_slug) DO UPDATE SET tag_slug = excluded.tag_slug
     RETURNING tag_id";

/// Attaches a tag to a post, tolerating the row already being there.
///
/// `DO NOTHING`, not `DO UPDATE`: `desired` may carry two labels sharing a slug
/// ([`post_tag_diff`] does not dedupe) and the first occurrence's casing must
/// win, so the existing row is left exactly as it is. Nothing reads a value
/// back, so there is no reason to force a row out of the conflict path here.
///
/// Bind order: `post_id, tag_id, tag_display`.
pub(crate) const INSERT_POST_TAG: &str = "INSERT INTO post_tags
     (post_id, tag_id, tag_display) VALUES ($1, $2, $3)
     ON CONFLICT (post_id, tag_id) DO NOTHING";

/// Drops one tag from a post, by slug, inside `set_post_tags`' transaction. The
/// SQL is identical on both dialects, so it is shared here per ADR-0019.
///
/// `rows_affected` is deliberately never checked by callers: the slug came from
/// the tags read in the same transaction, so "no row deleted" is not an error.
pub(crate) const DELETE_POST_TAG_BY_SLUG: &str = "DELETE FROM post_tags
     WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)";

/// The slug-level difference between a post's existing tags and a desired set
/// of display tokens, as computed by [`post_tag_diff`].
///
/// Borrows from both inputs. Applied by `set_post_tags` inside its transaction;
/// no caller performs the writes itself (#771).
pub(crate) struct PostTagDiff<'a> {
    /// Labels to add (their slug is not already present on the post).
    ///
    /// **Slug-ordered, contractually.** [`UPSERT_TAG_RETURNING_ID`] locks each
    /// `tags` row until commit, so applying these in a caller-supplied order lets
    /// two concurrent reconciles deadlock on Postgres (#876). The order is stable,
    /// so two labels sharing a slug keep their input order and the first
    /// occurrence's casing wins. Do not re-sort or re-shuffle at the call site.
    pub to_add: Vec<&'a TagLabel>,
    /// Existing tags to remove (their slug is not in the desired set).
    pub to_remove: Vec<&'a Tag>,
}

/// Diffs a post's `existing` tags against a `desired` set of [`TagLabel`]s.
///
/// Tagging is keyed on slug, so a desired label is "to add" only when no
/// existing tag shares its slug, and an existing tag is "to remove" only when
/// no desired label maps to its slug. Each `desired` label is already valid (its
/// `FromStr` ran at the boundary), so nothing is skipped here. Re-applying an
/// existing tag with different display casing is a no-op (the existing row's
/// casing is preserved by storage).
///
/// This is the pure core of `set_post_tags`, which applies the result inside its
/// own transaction on both dialects (#771).
#[must_use]
pub(crate) fn post_tag_diff<'a>(
    existing: &'a [PostTag],
    desired: &'a [TagLabel],
) -> PostTagDiff<'a> {
    let existing_slugs: HashSet<Tag> = existing.iter().map(|tag| tag.tag_slug.clone()).collect();
    let desired_slugs: HashSet<Tag> = desired.iter().map(TagLabel::slug).collect();

    let mut to_add: Vec<&'a TagLabel> = desired
        .iter()
        .filter(|label| !existing_slugs.contains(&label.slug()))
        .collect();
    // Slug order, so every transaction takes `tags` row locks in the same order —
    // caller-supplied order can deadlock concurrent reconciles on Postgres (#876,
    // docs/adr/0125-slug-ordered-tag-lock-acquisition.md).
    //
    // `sort_by_key`, not `sort_unstable_by_key`: `desired` may carry two labels
    // sharing a slug and the FIRST occurrence's casing must still win, which
    // `set_post_tags_is_idempotent_and_absorbs_duplicate_slugs` asserts.
    to_add.sort_by_key(|label| label.slug());
    let to_remove = existing
        .iter()
        .filter(|tag| !desired_slugs.contains(&tag.tag_slug))
        .map(|tag| &tag.tag_slug)
        .collect();

    PostTagDiff { to_add, to_remove }
}

/// Attaches the requested tags to a newly-created Post in canonical slug order.
///
/// Creation has no old child state to reconcile, but tag upserts still lock rows
/// on `PostgreSQL`. Sorting stably makes overlapping creates acquire those locks
/// in the same order while retaining the first spelling for duplicate slugs.
pub(crate) async fn insert_post_tags<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    desired: &[TagLabel],
) -> Result<()>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> Tag: Encode<'q, DB> + Type<DB>,
    for<'q> TagLabel: Encode<'q, DB> + Type<DB>,
    for<'q> TagId: Decode<'q, DB> + Type<DB>,
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let mut ordered = desired.to_vec();
    ordered.sort_by_key(TagLabel::slug);
    for label in ordered {
        let slug = label.slug();
        let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
            .bind_storage(&slug)
            .fetch_one(&mut *conn)
            .await?;
        sqlx::query(INSERT_POST_TAG)
            .bind_storage(post_id)
            .bind_storage(tag_id)
            .bind_storage(&label)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post_tag(slug: &str, display: &str) -> PostTag {
        PostTag {
            post_id: PostId::from(1),
            tag_id: TagId::from(0),
            tag_slug: slug.parse().expect("valid tag"),
            tag_display: display.parse().expect("valid tag label"),
        }
    }

    #[test]
    fn post_tag_diff_adds_removes_keeps() {
        let existing = vec![post_tag("rust", "Rust"), post_tag("leptos", "Leptos")];
        let desired: Vec<TagLabel> = vec![parse_tag_label("Rust"), parse_tag_label("wasm")];

        let diff = post_tag_diff(&existing, &desired);

        let added: Vec<String> = diff.to_add.iter().map(ToString::to_string).collect();
        assert_eq!(added, vec!["wasm".to_string()]);
        let removed: Vec<String> = diff.to_remove.iter().map(ToString::to_string).collect();
        assert_eq!(removed, vec!["leptos".to_string()]);
    }

    #[test]
    fn post_tag_diff_orders_additions_by_slug_stably() {
        let existing: Vec<PostTag> = vec![];
        let desired: Vec<TagLabel> = vec![
            parse_tag_label("wasm"),
            parse_tag_label("Nix"),
            parse_tag_label("NIX"),
            parse_tag_label("actix"),
        ];

        let diff = post_tag_diff(&existing, &desired);

        let added: Vec<String> = diff.to_add.iter().map(ToString::to_string).collect();
        assert_eq!(
            added,
            vec![
                "actix".to_string(),
                "Nix".to_string(),
                "NIX".to_string(),
                "wasm".to_string(),
            ],
            "additions are slug-ordered, and the duplicate slug keeps its input order"
        );
    }

    fn parse_tag_label(value: &str) -> TagLabel {
        value.parse().expect("valid tag label")
    }
}
