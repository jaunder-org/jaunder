//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use crate::ids::{AudienceId, ChannelId, UserId};

// Every string-backed enum here is a closed string enum (`#[text_enum]`, ADR-0075 as
// amended by #746): the attribute injects strum's `AsRefStr`/`Display`/`EnumString`/
// `IntoStaticStr` with the wire token as the snake_case variant name, and generates the
// named parse error plus the serde bridge. `AudienceBase` is wire-facing; the three
// below are FK-normalized — storage binds their token as a typed `&'static str` into a
// lookup column.
//
// `TargetKind` additionally takes `sqlx` (#728). FK-normalization is a fact about the
// *write* side — the column stores a `kind_id` — but the read side joins `target_kinds`
// and gets the **name back as text**, so the decode direction wants the bridge even
// though the bind direction does not. `Channel` and `SubscriptionStatus` are not read
// back that way and so stay bridge-less; when one of them is, it takes the flag too.
//
// Those three gain `Serialize`/`Deserialize` they do not currently need, which is the
// price of one convention rather than two (#746 D12). Note the cost: their tokens are a
// *storage encoding*, and the absence of serde used to be a compile-time barrier before
// they could become a wire contract. If that barrier is ever wanted back, the fix is a
// `no_serde` option on the attribute, not an exemption from it.
#[macros::text_enum(error = InvalidChannel, message = "channel must be \"local\"")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum Channel {
    Local,
}

#[macros::text_enum(
    error = InvalidSubscriptionStatus,
    message = "subscription status must be \"active\", \"pending\", or \"blocked\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum SubscriptionStatus {
    Active,
    Pending,
    Blocked,
}

#[macros::text_enum(
    sqlx,
    error = InvalidTargetKind,
    message = "audience target kind must be \"public\", \"subscribers\", or \"named\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum TargetKind {
    Public,
    Subscribers,
    Named,
}

// The mutually-exclusive built-in audience base chosen in the editor / API — the
// typed form of the audience-picker's `base`. Composes with named audiences by
// union except for `Private` (author-only), which is the safe, non-widening
// `Default` (faithful to the prior empty-string -> author-only fall-through). #499.
#[macros::text_enum(
    error = InvalidAudienceBase,
    message = "audience must be \"private\", \"public\", or \"subscribers\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[strum(serialize_all = "snake_case")]
pub enum AudienceBase {
    #[default]
    Private,
    Public,
    Subscribers,
}

/// Who is reading. Wider than Layer A needs (only `Anonymous` and `Local` are
/// constructed today) so non-local channels need no signature change in Layers
/// B/C. `Remote`'s `subscriber_ref` makes this non-`Copy`. See ADR-0020.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerIdentity {
    Anonymous,
    /// A logged-in local account. Locality is carried by the *variant*, not by
    /// the shape of a string: the author branch of the resolution filter fires
    /// on this and nothing else (#6).
    ///
    /// It carries no `channel_id` because a local viewer's channel is not a
    /// free parameter — it is always the `local` row, which the queries that
    /// need it resolve inline in SQL (#6).
    Local {
        user_id: UserId,
    },
    /// A non-local channel identity (an `ActivityPub` actor, an email address).
    /// Its `subscriber_ref` is opaque — never a local user id, whatever it
    /// happens to parse as.
    Remote {
        channel_id: ChannelId,
        subscriber_ref: String,
    },
}

impl ViewerIdentity {
    /// Local viewer constructor used by Layer A: a logged-in account on the
    /// `local` channel.
    #[must_use]
    pub fn local(user_id: UserId) -> Self {
        Self::Local { user_id }
    }
}

/// How a local account appears in `subscriptions.subscriber_ref`: its user id
/// in decimal.
///
/// `subscriber_ref` is a `TEXT` column shared by every channel, so a local
/// account has to be spelled into it somehow. That spelling is a *storage
/// encoding*, not a property of [`UserId`] — the write path (`subscribe` /
/// `unsubscribe`) and the read paths (the resolution filter, `is_subscriber`)
/// must agree on it exactly, or a subscription silently stops matching. This
/// is the one place it is defined; call it rather than spelling it again (#6).
#[must_use]
pub fn local_subscriber_ref(user_id: UserId) -> String {
    user_id.to_string()
}

/// The local user id of an account viewer, for *display* of owner controls.
///
/// This is the same identity the web `viewer_identity()` extractor resolves,
/// projected back to a bare `user_id`: `Some(user_id)` for a `local` channel
/// viewer, `None` for anonymous. Filtering itself lives in the store query; this
/// is used only to decide whether to render author-only UI affordances.
#[must_use]
pub fn viewer_user_id(viewer: &ViewerIdentity) -> Option<UserId> {
    match viewer {
        // Only a local account has a local user id. A remote `subscriber_ref`
        // is opaque — that it parses as an integer says nothing about who it
        // is, so it never projects to a user id (#6).
        ViewerIdentity::Local { user_id } => Some(*user_id),
        ViewerIdentity::Remote { .. } | ViewerIdentity::Anonymous => None,
    }
}

/// What a post is addressed to, as chosen in the editor / API.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AudienceTarget {
    Public,
    Private,
    Subscribers,
    Named(AudienceId),
}

/// Parses a stored site-wide default audience. Only the built-ins are valid:
/// [`AudienceTarget::Named`] is per-author and has no instance-wide form, so it is
/// rejected here (the caller falls back to `Public`).
///
/// It lives beside [`AudienceTarget`] rather than in `storage` because the config-key
/// registry (`crate::config_key`) needs it as a validator and `storage` is downstream of
/// `common`, not upstream (#687).
#[must_use]
pub fn parse_default_audience(value: &str) -> Option<AudienceTarget> {
    match value.trim() {
        "public" => Some(AudienceTarget::Public),
        "subscribers" => Some(AudienceTarget::Subscribers),
        "private" => Some(AudienceTarget::Private),
        _ => None,
    }
}

/// String form for a site-wide default audience — the inverse of
/// [`parse_default_audience`]. [`AudienceTarget::Named`] has no instance-wide form, so it
/// collapses to `public`.
#[must_use]
pub fn default_audience_str(audience: &AudienceTarget) -> &'static str {
    match audience {
        AudienceTarget::Public | AudienceTarget::Named(_) => "public",
        AudienceTarget::Subscribers => "subscribers",
        AudienceTarget::Private => "private",
    }
}

/// The audience-picker selection as it crosses the server-fn boundary.
///
/// `base` is the mutually-exclusive built-in ([`AudienceBase::Public`],
/// [`AudienceBase::Private`], or [`AudienceBase::Subscribers`]); `named` is the
/// set of selected named-audience ids. The two compose by UNION except for
/// [`AudienceBase::Private`], which is author-only and cannot combine with
/// anything — a `Private` base discards `named`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct AudienceSelection {
    pub base: AudienceBase,
    pub named: Vec<AudienceId>,
}

/// Translates an [`AudienceSelection`] into the `Vec<AudienceTarget>` the
/// storage layer persists.
///
/// - [`AudienceBase::Public`] / [`AudienceBase::Subscribers`] → the built-in
///   target, in union with one `Named(id)` per selected named audience.
/// - [`AudienceBase::Private`] → an empty vec (author-only); the named set is
///   ignored, since `Private` cannot combine with other targets.
#[must_use]
pub fn audience_selection_to_targets(selection: &AudienceSelection) -> Vec<AudienceTarget> {
    let base = match selection.base {
        AudienceBase::Public => Some(AudienceTarget::Public),
        AudienceBase::Subscribers => Some(AudienceTarget::Subscribers),
        // Private is author-only: no built-in target, and named is dropped below.
        AudienceBase::Private => None,
    };
    let Some(base) = base else {
        // Private/author-only: no rows, named selection ignored.
        return Vec::new();
    };
    std::iter::once(base)
        .chain(selection.named.iter().copied().map(AudienceTarget::Named))
        .collect()
}

/// Resolves an optional picker selection to the targets to persist. An absent
/// selection defaults to `[Public]` — the historical behavior and the safe
/// default for non-editor callers that omit the field on the wire.
#[must_use]
pub fn audience_targets_or_public(selection: Option<&AudienceSelection>) -> Vec<AudienceTarget> {
    selection.map_or_else(
        || vec![AudienceTarget::Public],
        audience_selection_to_targets,
    )
}

/// Translates a post's persisted `Vec<AudienceTarget>` into the picker's
/// [`AudienceSelection`] (the inverse of [`audience_selection_to_targets`],
/// for pre-selecting the editor).
///
/// The built-in base is [`AudienceBase::Public`]/[`AudienceBase::Subscribers`]
/// when that target is present, otherwise [`AudienceBase::Private`] (covering
/// both an explicit `Private` and an empty targeting). Every `Named(id)` becomes
/// an entry in `named`.
#[must_use]
pub fn targets_to_audience_selection(targets: &[AudienceTarget]) -> AudienceSelection {
    let mut base = AudienceBase::Private;
    let mut named = Vec::new();
    for target in targets {
        match target {
            AudienceTarget::Public => base = AudienceBase::Public,
            AudienceTarget::Subscribers => base = AudienceBase::Subscribers,
            AudienceTarget::Named(id) => named.push(*id),
            AudienceTarget::Private => {}
        }
    }
    AudienceSelection { base, named }
}

/// Admission seam: decides the initial status of a new subscription. Layer A
/// auto-approves (`Active`); M13 swaps the one impl below for an approval gate.
pub trait SubscriptionPolicy: Send + Sync {
    fn initial_status(
        &self,
        author_user_id: UserId,
        channel_id: ChannelId,
        subscriber_ref: &str,
    ) -> SubscriptionStatus;
}

/// Layer A NOOP policy: every subscription is admitted as `Active`.
pub struct OpenSubscriptionPolicy;

impl SubscriptionPolicy for OpenSubscriptionPolicy {
    fn initial_status(&self, _a: UserId, _c: ChannelId, _r: &str) -> SubscriptionStatus {
        SubscriptionStatus::Active // Layer A NOOP auto-approve; M13 swaps this here.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_kind_roundtrips() {
        for k in [
            TargetKind::Public,
            TargetKind::Subscribers,
            TargetKind::Named,
        ] {
            assert_eq!(TargetKind::try_from(k.as_ref()), Ok(k));
        }
        let err = TargetKind::try_from("private").unwrap_err();
        assert_eq!(
            err.to_string(),
            "audience target kind must be \"public\", \"subscribers\", or \"named\""
        );
    }

    #[test]
    fn display_matches_as_str() {
        // Covers the macro-generated `Display` impl for every enum, including
        // the `SubscriptionStatus` variants reserved for later milestones that
        // have no lookup row (and thus no bijection-test exposure) yet.
        assert_eq!(Channel::Local.to_string(), Channel::Local.as_ref());
        for s in [
            SubscriptionStatus::Active,
            SubscriptionStatus::Pending,
            SubscriptionStatus::Blocked,
        ] {
            assert_eq!(s.to_string(), s.as_ref());
            assert_eq!(SubscriptionStatus::try_from(s.as_ref()), Ok(s));
        }
        for k in [
            TargetKind::Public,
            TargetKind::Subscribers,
            TargetKind::Named,
        ] {
            assert_eq!(k.to_string(), k.as_ref());
        }
        for b in [
            AudienceBase::Public,
            AudienceBase::Subscribers,
            AudienceBase::Private,
        ] {
            assert_eq!(b.to_string(), b.as_ref());
            assert_eq!(AudienceBase::try_from(b.as_ref()), Ok(b));
        }
    }

    #[test]
    fn fk_enums_round_trip_through_serde() {
        // These three gained serde with #746 D12 and had no serde coverage before, so
        // without this a broken `Serialize` would pass every other assertion here.
        assert_eq!(
            serde_json::from_str::<Channel>(&serde_json::to_string(&Channel::Local).unwrap())
                .unwrap(),
            Channel::Local
        );
        for s in [
            SubscriptionStatus::Active,
            SubscriptionStatus::Pending,
            SubscriptionStatus::Blocked,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_ref()));
            assert_eq!(
                serde_json::from_str::<SubscriptionStatus>(&json).unwrap(),
                s
            );
        }
        for k in [
            TargetKind::Public,
            TargetKind::Subscribers,
            TargetKind::Named,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.as_ref()));
            assert_eq!(serde_json::from_str::<TargetKind>(&json).unwrap(), k);
        }
    }

    #[test]
    fn channel_rejects_unknown_with_named_error() {
        let err = Channel::try_from("bogus").unwrap_err();
        assert_eq!(err.to_string(), "channel must be \"local\"");
    }

    #[test]
    fn subscription_status_rejects_unknown_with_named_error() {
        let err = SubscriptionStatus::try_from("bogus").unwrap_err();
        assert_eq!(
            err.to_string(),
            "subscription status must be \"active\", \"pending\", or \"blocked\""
        );
    }

    #[test]
    fn audience_base_serializes_to_lowercase_literal() {
        assert_eq!(
            serde_json::to_string(&AudienceBase::Public).unwrap(),
            "\"public\""
        );
        assert_eq!(
            serde_json::to_string(&AudienceBase::Subscribers).unwrap(),
            "\"subscribers\""
        );
        assert_eq!(
            serde_json::to_string(&AudienceBase::Private).unwrap(),
            "\"private\""
        );
    }

    #[test]
    fn audience_base_deserializes_from_literal() {
        for v in [
            AudienceBase::Public,
            AudienceBase::Subscribers,
            AudienceBase::Private,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(serde_json::from_str::<AudienceBase>(&json).unwrap(), v);
        }
    }

    #[test]
    fn audience_base_deserialize_rejects_unknown() {
        assert!(serde_json::from_str::<AudienceBase>("\"bogus\"").is_err());
    }

    #[test]
    fn audience_base_rejects_unknown_with_named_error() {
        let err = "bogus".parse::<AudienceBase>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "audience must be \"private\", \"public\", or \"subscribers\""
        );
    }

    #[test]
    fn audience_base_default_is_private() {
        assert_eq!(AudienceBase::default(), AudienceBase::Private);
    }

    #[test]
    fn open_policy_returns_active() {
        assert_eq!(
            OpenSubscriptionPolicy.initial_status(UserId::from(1), ChannelId::from(1), "1"),
            SubscriptionStatus::Active
        );
    }

    #[test]
    fn viewer_local_constructor_builds_a_local_viewer() {
        let viewer = ViewerIdentity::local(UserId::from(42));
        assert_eq!(
            viewer,
            ViewerIdentity::Local {
                user_id: UserId::from(42),
            }
        );
    }

    #[test]
    fn viewer_user_id_projects_local_channel_to_user_id() {
        assert_eq!(
            viewer_user_id(&ViewerIdentity::local(UserId::from(42))),
            Some(UserId::from(42))
        );
    }

    #[test]
    fn viewer_user_id_is_none_for_anonymous() {
        assert_eq!(viewer_user_id(&ViewerIdentity::Anonymous), None);
    }

    #[test]
    fn local_subscriber_ref_is_the_user_id_in_decimal() {
        // Locks the storage encoding the subscription write path and both read
        // paths must agree on; a change here silently unmatches existing rows.
        assert_eq!(local_subscriber_ref(UserId::from(42)), "42");
    }

    #[test]
    fn viewer_user_id_is_none_for_a_remote_viewer_with_a_numeric_ref() {
        // The #6 hole in its second form: a remote ref that happens to be the
        // decimal form of a local user id must not project to that user, or the
        // owner-only controls render for a viewer who is not the owner.
        let impostor = ViewerIdentity::Remote {
            channel_id: ChannelId::from(2),
            subscriber_ref: "42".to_owned(),
        };
        assert_eq!(viewer_user_id(&impostor), None);
    }

    #[test]
    fn viewer_user_id_is_none_for_a_remote_actor_uri() {
        // A remote identity is not a local account, so it has no local user id
        // to project to and renders no owner-only affordances.
        assert_eq!(
            viewer_user_id(&ViewerIdentity::Remote {
                channel_id: ChannelId::from(2),
                subscriber_ref: "https://remote.example/users/alice".to_owned(),
            }),
            None
        );
    }

    fn selection(base: AudienceBase, named: &[AudienceId]) -> AudienceSelection {
        AudienceSelection {
            base,
            named: named.to_vec(),
        }
    }

    #[test]
    fn public_selection_maps_to_public_target() {
        assert_eq!(
            audience_selection_to_targets(&selection(AudienceBase::Public, &[])),
            vec![AudienceTarget::Public]
        );
    }

    #[test]
    fn subscribers_selection_maps_to_subscribers_target() {
        assert_eq!(
            audience_selection_to_targets(&selection(AudienceBase::Subscribers, &[])),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn public_plus_named_unions() {
        assert_eq!(
            audience_selection_to_targets(&selection(
                AudienceBase::Public,
                &[AudienceId::from(5), AudienceId::from(9)]
            )),
            vec![
                AudienceTarget::Public,
                AudienceTarget::Named(AudienceId::from(5)),
                AudienceTarget::Named(AudienceId::from(9)),
            ]
        );
    }

    #[test]
    fn private_selection_is_empty_and_ignores_named() {
        // Private cannot combine with anything; named ids are dropped.
        assert!(
            audience_selection_to_targets(&selection(
                AudienceBase::Private,
                &[AudienceId::from(5)]
            ))
            .is_empty()
        );
    }

    #[test]
    fn absent_selection_defaults_to_public() {
        assert_eq!(
            audience_targets_or_public(None),
            vec![AudienceTarget::Public]
        );
        // A present selection is translated normally.
        assert_eq!(
            audience_targets_or_public(Some(&selection(AudienceBase::Subscribers, &[]))),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn targets_round_trip_through_selection() {
        // Edit round-trip: persisted targets -> selection -> targets.
        let targets = vec![
            AudienceTarget::Subscribers,
            AudienceTarget::Named(AudienceId::from(3)),
        ];
        let sel = targets_to_audience_selection(&targets);
        assert_eq!(
            sel,
            selection(AudienceBase::Subscribers, &[AudienceId::from(3)])
        );
        assert_eq!(audience_selection_to_targets(&sel), targets);

        // Public round-trips through the picker.
        let sel = targets_to_audience_selection(&[AudienceTarget::Public]);
        assert_eq!(sel, selection(AudienceBase::Public, &[]));
        assert_eq!(
            audience_selection_to_targets(&sel),
            vec![AudienceTarget::Public]
        );

        // An explicit Private element yields a private selection.
        assert_eq!(
            targets_to_audience_selection(&[AudienceTarget::Private]),
            selection(AudienceBase::Private, &[])
        );

        // No rows (private) round-trips to a private selection and back to empty.
        let empty: Vec<AudienceTarget> = Vec::new();
        let sel = targets_to_audience_selection(&empty);
        assert_eq!(sel, selection(AudienceBase::Private, &[]));
        assert!(audience_selection_to_targets(&sel).is_empty());
    }
}
