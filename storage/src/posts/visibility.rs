//! Visibility resolution and audience persistence for posts.

use sqlx::{Database, Encode, Executor, Result, Type};

use crate::posts::models::PostRecord;
use crate::posts::store::PostDialect;
use crate::sql::QueryStorageExt;
use common::ids::{AudienceId, ChannelId, PostId, UserId};
use common::visibility::{self, AudienceTarget, SubscriberRef, TargetKind, ViewerIdentity};

/// A visibility predicate generated exclusively by [`resolution_where`].
///
/// The SQL text is intentionally opaque outside this module: callers may splice
/// the generated predicate into a larger fixed query, but cannot manufacture a
/// value accepted by narrow query constructors.
pub(crate) struct ResolutionWhere(String);

impl std::fmt::Display for ResolutionWhere {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
impl ResolutionWhere {
    fn as_str(&self) -> &str {
        &self.0
    }
}

/// The viewer-resolution binds folded into a read query's `WHERE`, in the exact
/// left-to-right order their placeholders appear in [`resolution_where`]'s
/// fragment. `subref` (and, where it is bound at all, `channel`) repeats —
/// subscribers branch, then named branch — because each occurrence gets its own
/// placeholder; see [`resolution_where`].
///
/// The enum mirrors [`ViewerIdentity`] rather than carrying three independent
/// `Option`s, because **bind arity is per-variant**: a `Local` viewer's channel
/// is resolved in SQL and so is not bound at all. Recovering that from three
/// `Option`s would mean reading `(Some, None, Some)` as "local" — an implicit
/// encoding of exactly the fact the variant already states.
pub(crate) enum ResolutionBinds {
    /// No viewer: all five placeholders bind SQL NULL, which makes every
    /// comparison *unknown* rather than true — see [`resolution_where`] for why
    /// that is what "this branch cannot match" means here.
    Anonymous,
    /// A local viewer: three binds — `author_id, subref, subref`. The channel is
    /// the seeded `local` row, resolved by subquery instead of bound.
    Local {
        /// `p.user_id = $author_id` — the author branch fires for this and only
        /// this variant.
        user_id: UserId,
        /// `s.subscriber_ref` for the subscribers/named branches: the viewer's
        /// user id in decimal, the form `subscribe_to` stores.
        subref: SubscriberRef,
    },
    /// A non-local viewer: five binds — `NULL, channel, subref, channel, subref`.
    /// The author placeholder binds NULL, so the author branch cannot fire (#6).
    Remote {
        /// `s.channel_id` for the subscribers/named `EXISTS` branches.
        channel: ChannelId,
        /// `s.subscriber_ref` for the subscribers/named branches.
        subref: SubscriberRef,
    },
}

/// The viewer-resolution predicate and its binds, for folding into a read
/// query's `WHERE`. A post is returned to `viewer` only if the viewer is the
/// author OR some targeted audience admits them. See ADR-0020, Task 13.
///
/// The fragment is emitted in full for every viewer; `Anonymous` is handled by
/// binding NULL for all three values, so it reduces to "public posts only"
/// without a second query shape. A NULL comparison is *unknown*, never true:
/// `p.user_id = NULL` cannot admit a post, and the `EXISTS` subqueries match no
/// row, so `EXISTS` is false. The fragment contains no `NOT`, and the caller
/// `AND`s it into a `WHERE`, where unknown filters the row out exactly as false
/// would — so NULL kills every non-`public` branch.
///
/// NULL binds make the unreachable branches follow from SQL comparison
/// semantics rather than from reserving sentinel IDs or subscriber references.
/// The predicate therefore stays correct independently of which concrete
/// values the live schema permits.
///
/// `start` is the next free `$n` index, and **the placeholder count is
/// per-variant**, so callers must thread the returned `next` rather than assume
/// `start + 5`:
///
/// - `Local` uses THREE (`$start`..`$start+2`): `author, subref, subref`. Its
///   channel is not a bind — a local viewer's channel is always the seeded
///   `local` row, so both subscription branches resolve it inline with an
///   uncorrelated subquery (`channels.name` is `NOT NULL UNIQUE`, so it yields at
///   most one row).
/// - `Anonymous` and `Remote` use FIVE (`$start`..`$start+4`):
///   `author, channel, subref, channel, subref`.
///
/// Either way the `channel`/`subref` pair appears once in the subscribers branch
/// and again in the named branch, and each bound occurrence gets its own number
/// so the binds are positional on both backends (`SQLite` accepts `$n` and binds
/// by position; see ADR-0019) — which is why the returned [`ResolutionBinds`]
/// carries `subref` once but the caller binds it **twice**. Returns
/// `(predicate, binds, next)` where `next` is the first free index after the
/// fragment. `predicate` is an opaque generated structural fragment; only this
/// function can construct it.
pub(crate) fn resolution_where(
    viewer: &ViewerIdentity,
    start: usize,
) -> (ResolutionWhere, ResolutionBinds, usize) {
    /// The seeded `local` channel, resolved in SQL rather than bound — see the
    /// doc above and ADR-0020.
    const LOCAL_CHANNEL: &str = "(SELECT channel_id FROM channels WHERE name = 'local')";

    let binds = match viewer {
        ViewerIdentity::Anonymous => ResolutionBinds::Anonymous,
        // Only a local viewer can be the author. Its channel is not carried at
        // all, because it can only ever be the `local` row, which the SQL
        // resolves for itself.
        ViewerIdentity::Local { user_id } => ResolutionBinds::Local {
            user_id: *user_id,
            subref: visibility::local_subscriber_ref(*user_id),
        },
        // A remote viewer is never the author, whatever its ref parses as: the
        // author bind stays NULL, so `p.user_id = NULL` is unknown and admits
        // nothing (#6). It can still be admitted by a subscription branch.
        ViewerIdentity::Remote {
            channel_id,
            subscriber_ref,
        } => ResolutionBinds::Remote {
            channel: *channel_id,
            subref: subscriber_ref.clone(),
        },
    };
    let author = start;
    // The channel slots are *expressions*, not necessarily placeholders, and the
    // ref slots renumber accordingly — hence the per-variant `next`.
    let (sub_channel, sub_refnum, named_channel, named_refnum, next) = match binds {
        ResolutionBinds::Local { .. } => (
            LOCAL_CHANNEL.to_owned(),
            format!("${}", start + 1),
            LOCAL_CHANNEL.to_owned(),
            format!("${}", start + 2),
            start + 3,
        ),
        ResolutionBinds::Anonymous | ResolutionBinds::Remote { .. } => (
            format!("${}", start + 1),
            format!("${}", start + 2),
            format!("${}", start + 3),
            format!("${}", start + 4),
            start + 5,
        ),
    };
    let sql = format!(
        "( p.user_id = ${author}
  OR EXISTS (
    SELECT 1 FROM post_audiences pa
    JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
    WHERE pa.post_id = p.post_id AND (
         tk.name = 'public'
      OR (tk.name = 'subscribers' AND EXISTS (
            SELECT 1 FROM subscriptions s JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE s.author_user_id = p.user_id AND s.channel_id = {sub_channel}
              AND s.subscriber_ref = {sub_refnum} AND st.name = 'active'))
      OR (tk.name = 'named' AND EXISTS (
            SELECT 1 FROM audience_members am
            JOIN subscriptions s ON s.subscription_id = am.subscription_id
            JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE am.audience_id = pa.audience_id AND s.channel_id = {named_channel}
              AND s.subscriber_ref = {named_refnum} AND st.name = 'active'))
  ))
)"
    );
    (ResolutionWhere(sql), binds, next)
}

impl ResolutionBinds {
    /// Binds this variant's resolution placeholders onto `query` in the exact
    /// fragment order — `author_id, channel, subref, channel, subref`, minus the
    /// two channel binds for [`ResolutionBinds::Local`], whose channel is
    /// resolved in SQL. The caller must have already bound everything to the left
    /// of the fragment, and must bind the query's trailing binds (e.g. `LIMIT`)
    /// afterward, at the index [`resolution_where`] returned.
    pub(crate) fn bind_onto<'q, DB>(
        &'q self,
        query: sqlx::query::QueryAs<'q, DB, PostRecord, DB::Arguments>,
    ) -> sqlx::query::QueryAs<'q, DB, PostRecord, DB::Arguments>
    where
        DB: Database,
        i64: Encode<'q, DB> + Type<DB>,
        &'q str: Encode<'q, DB> + Type<DB>,
        &'q SubscriberRef: Encode<'q, DB> + Type<DB>,
        Option<&'q SubscriberRef>: Encode<'q, DB> + Type<DB>,
        // sqlx implements `Encode for Option<T>` per concrete database (the
        // `impl_encode_for_option!` macro), not blanket over a generic `DB`, so
        // each NULL-able bind's type has to be restated here — and, per ADR-0019,
        // again on every caller.
        Option<UserId>: Encode<'q, DB> + Type<DB>,
        Option<ChannelId>: Encode<'q, DB> + Type<DB>,
        Option<&'q str>: Encode<'q, DB> + Type<DB>,
    {
        match self {
            Self::Anonymous => query
                .bind_storage(None::<UserId>)
                .bind_storage(None::<ChannelId>)
                .bind_storage(None::<&SubscriberRef>)
                .bind_storage(None::<ChannelId>)
                .bind_storage(None::<&SubscriberRef>),
            Self::Local { user_id, subref } => query
                .bind_storage(Some(*user_id))
                .bind_storage(Some(subref))
                .bind_storage(Some(subref)),
            Self::Remote { channel, subref } => query
                .bind_storage(None::<UserId>)
                .bind_storage(Some(*channel))
                .bind_storage(Some(subref))
                .bind_storage(Some(*channel))
                .bind_storage(Some(subref)),
        }
    }
}

/// Maps an [`AudienceTarget`] to its `post_audiences` row shape:
/// `(target kind, audience_id)`. `Private` produces no row.
pub(crate) fn audience_target_row(
    target: &AudienceTarget,
) -> Option<(TargetKind, Option<AudienceId>)> {
    match target {
        AudienceTarget::Public => Some((TargetKind::Public, None)),
        AudienceTarget::Subscribers => Some((TargetKind::Subscribers, None)),
        AudienceTarget::Named(id) => Some((TargetKind::Named, Some(*id))),
        AudienceTarget::Private => None,
    }
}

/// Maps a `post_audiences` row `(target_kind name, audience_id)` back to its
/// [`AudienceTarget`] — the inverse of [`audience_target_row`], used by
/// [`PostStorage::get_post_audiences`].
///
/// `public` → [`AudienceTarget::Public`], `subscribers` →
/// [`AudienceTarget::Subscribers`], `named` (with an id) →
/// [`AudienceTarget::Named`].
///
/// **Returns `Option`, for one reason only.** A `named` row whose `audience_id` is
/// NULL has no target to build, so it is dropped — asserted below. An unrecognised
/// kind name never reaches this function: the column decodes as `TargetKind`
/// (#728), so it is a `ColumnDecode` error at the query boundary rather than a
/// silent drop here.
pub(crate) fn audience_target_from_row(
    kind: TargetKind,
    audience_id: Option<AudienceId>,
) -> Option<AudienceTarget> {
    match kind {
        TargetKind::Public => Some(AudienceTarget::Public),
        TargetKind::Subscribers => Some(AudienceTarget::Subscribers),
        TargetKind::Named => audience_id.map(AudienceTarget::Named),
    }
}

/// Compares audience collections as normalized relation rows, ignoring caller
/// order, duplicate selections, and `Private`'s deliberate absence of a row.
pub(crate) fn audiences_are_equal(
    existing: &[(TargetKind, Option<AudienceId>)],
    desired: &[AudienceTarget],
) -> bool {
    let mut normalized = Vec::new();
    for target in desired {
        if let Some(row) = audience_target_row(target)
            && !normalized.contains(&row)
        {
            normalized.push(row);
        }
    }
    existing.len() == normalized.len() && existing.iter().all(|row| normalized.contains(row))
}

/// Replaces a post's `post_audiences` rows to exactly match `audiences`.
///
/// Deletes every existing row for `post_id`, then inserts one row per targeting
/// entry (`Public`/`Subscribers` carry a NULL `audience_id`; `Named(id)` carries
/// the id; `Private` and an empty vec leave the post with no rows). Runs on the
/// caller's executor so it shares the create/update transaction. See ADR-0020.
pub(crate) async fn replace_post_audiences<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    audiences: &[AudienceTarget],
) -> Result<()>
where
    DB: PostDialect,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> TargetKind: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    sqlx::query(DB::DELETE_POST_AUDIENCES)
        .bind_storage(post_id)
        .execute(&mut *conn)
        .await?;
    for target in audiences {
        if let Some((kind, audience_id)) = audience_target_row(target) {
            sqlx::query(DB::INSERT_POST_AUDIENCE)
                .bind_storage(post_id)
                .bind_storage(audience_id)
                .bind_storage(kind)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ResolutionBinds, audience_target_from_row, resolution_where};
    use common::ids::{AudienceId, ChannelId, UserId};
    use common::visibility::{AudienceTarget, TargetKind, ViewerIdentity};
    use rstest::*;

    /// A local viewer's channel is not a bind: both subscription branches resolve
    /// the seeded `local` row inline, so the fragment spends three placeholders
    /// (author, ref, ref) and `next` is `start + 3` — the property every call site
    /// depends on by threading the returned index rather than assuming `+5` (#6).
    #[test]
    fn resolution_where_resolves_the_local_channel_in_sql_for_a_local_viewer() {
        let viewer = ViewerIdentity::Local {
            user_id: UserId::from(7),
        };
        let (sql, binds, next) = resolution_where(&viewer, 2);
        assert!(matches!(binds, ResolutionBinds::Local { .. }));
        let sql = sql.as_str();
        assert_eq!(next, 5, "three placeholders consumed from $2: {sql}");
        assert_eq!(
            sql.matches("(SELECT channel_id FROM channels WHERE name = 'local')")
                .count(),
            2,
            "both the subscribers and named branches resolve the channel: {sql}"
        );
        assert!(sql.contains("p.user_id = $2"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $3"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $4"), "{sql}");
        assert!(
            !sql.contains("$5"),
            "no fourth placeholder is emitted: {sql}"
        );
        assert!(
            !sql.contains("99"),
            "the carried channel id is ignored: {sql}"
        );
    }

    /// The counterpart: `Anonymous` and `Remote` keep the five-placeholder shape,
    /// binding the channel rather than resolving it.
    #[rstest]
    #[case::anonymous(ViewerIdentity::Anonymous)]
    #[case::remote(ViewerIdentity::Remote {
        channel_id: ChannelId::from(2),
        subscriber_ref: "7".parse().unwrap(),
    })]
    fn resolution_where_binds_the_channel_for_a_non_local_viewer(#[case] viewer: ViewerIdentity) {
        let (sql, _binds, next) = resolution_where(&viewer, 2);
        let sql = sql.as_str();
        assert_eq!(next, 7, "five placeholders consumed from $2: {sql}");
        assert!(
            !sql.contains("name = 'local'"),
            "a non-local viewer never resolves the local channel: {sql}"
        );
        assert!(sql.contains("s.channel_id = $3"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $4"), "{sql}");
        assert!(sql.contains("s.channel_id = $5"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $6"), "{sql}");
    }

    #[test]
    fn audience_target_from_row_maps_every_kind() {
        // Each lookup-table kind maps to its target; `named` carries the id.
        assert_eq!(
            audience_target_from_row(TargetKind::Public, None),
            Some(AudienceTarget::Public)
        );
        assert_eq!(
            audience_target_from_row(TargetKind::Subscribers, None),
            Some(AudienceTarget::Subscribers)
        );
        assert_eq!(
            audience_target_from_row(TargetKind::Named, Some(AudienceId::from(7))),
            Some(AudienceTarget::Named(AudienceId::from(7)))
        );
        // A `named` row missing its id is dropped — the only reason this returns
        // `Option`.
        assert_eq!(audience_target_from_row(TargetKind::Named, None), None);
        // An unrecognised kind name is not expressible here: the parameter is a
        // `TargetKind`, so a bad name cannot get this far.
        // `get_post_audiences_rejects_an_unknown_target_kind` covers it at the
        // boundary where it surfaces.
    }
}
