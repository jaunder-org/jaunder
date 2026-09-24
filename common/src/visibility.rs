//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

use crate::ids::{AudienceId, ChannelId, UserId};

// Every string-backed enum here is a closed string enum (`#[text_enum]`, ADR-0075 as
// amended by #746): the attribute injects strum's `AsRefStr`/`Display`/`EnumString`/
// `IntoStaticStr` with the wire token as the snake_case variant name, and generates the
// named parse error plus the serde bridge. The three below are FK-normalized —
// storage binds their token as a typed `&'static str` into a
// lookup column.
//
// `TargetKind` additionally takes `sqlx` (#728). FK-normalization is a fact about the
// *write* side — the column stores a `kind_id` — but the read side joins `target_kinds`
// and gets the **name back as text**, so the decode direction wants the bridge even
// though the bind direction does not. `Channel` and `SubscriptionStatus` are not read
// back that way and so stay bridge-less; when one of them is, it takes the flag too.
//
// Those three gain `Serialize`/`Deserialize` they do not currently need — the price of
// one convention rather than two (#746 D12). Note the cost: their tokens are a
// *storage encoding*, and serde's presence removes the compile-time barrier that would
// otherwise stop them becoming a wire contract. If that barrier is ever wanted back,
// the fix is a `no_serde` option on the attribute, not an exemption from it.
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

/// The stored `subscriptions.subscriber_ref` value.
///
/// This is a storage-domain value, not display text. It is interpreted only with
/// its `channel_id`: for the seeded `local` channel it is the decimal local
/// [`UserId`] spelling, while remote channels own their own opaque namespace.
/// Untrusted strings enter through [`FromStr`], which rejects blank references
/// without normalizing accepted spellings.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct SubscriberRef(String);

/// Error returned when a string cannot be parsed as a [`SubscriberRef`].
#[derive(Debug, Error)]
#[error("subscriber reference must not be blank")]
pub struct InvalidSubscriberRef;

impl FromStr for SubscriberRef {
    type Err = InvalidSubscriberRef;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.trim().is_empty() {
            return Err(InvalidSubscriberRef);
        }
        Ok(Self(value.to_owned()))
    }
}

/// Persisted subscription identity.
///
/// The `subscriber_ref` leaf has meaning only inside its channel namespace. Keep
/// the pair together across storage boundaries so local-user spellings cannot be
/// confused with remote subscriber references.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubscriberIdentity {
    pub channel_id: ChannelId,
    pub subscriber_ref: SubscriberRef,
}

impl SubscriberIdentity {
    #[must_use]
    pub fn new(channel_id: ChannelId, subscriber_ref: SubscriberRef) -> Self {
        Self {
            channel_id,
            subscriber_ref,
        }
    }
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
        subscriber_ref: SubscriberRef,
    },
}

#[must_use]
pub fn local_subscriber_identity(channel_id: ChannelId, user_id: UserId) -> SubscriberIdentity {
    SubscriberIdentity::new(channel_id, local_subscriber_ref(user_id))
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
pub fn local_subscriber_ref(user_id: UserId) -> SubscriberRef {
    // A decimal integer spelling is necessarily non-blank, so `UserId` is the
    // proof that this private construction preserves the domain invariant.
    SubscriberRef(user_id.to_string())
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

/// The closed instance-wide audience for Posts without an explicit audience.
///
/// Unlike [`AudienceTarget`], this cannot name a per-author audience.
#[macros::text_enum(
    sqlx,
    error = InvalidDefaultAudience,
    message = "default audience must be \"public\", \"subscribers\", or \"private\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum DefaultAudience {
    Public,
    Subscribers,
    Private,
}

/// What a post is addressed to, as chosen in the editor / API.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AudienceTarget {
    Public,
    Private,
    Subscribers,
    Named(AudienceId),
}

/// Widens an instance-wide default at the per-Post targeting boundary.
impl From<DefaultAudience> for AudienceTarget {
    fn from(audience: DefaultAudience) -> Self {
        match audience {
            DefaultAudience::Public => Self::Public,
            DefaultAudience::Subscribers => Self::Subscribers,
            DefaultAudience::Private => Self::Private,
        }
    }
}

/// The complete audience-picker selection as it crosses the web server-fn boundary.
///
/// Built-in and Named targets compose by union. No selected targets is Private;
/// its empty target set cannot be combined with anything else. Reject the old
/// `base` wire shape rather than silently treating it as an empty selection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AudienceSelection {
    pub public: bool,
    pub subscribers: bool,
    pub named: Vec<AudienceId>,
}

/// Translates every checked target into the set persisted by storage.
#[must_use]
pub fn audience_selection_to_targets(selection: &AudienceSelection) -> Vec<AudienceTarget> {
    selection
        .public
        .then_some(AudienceTarget::Public)
        .into_iter()
        .chain(selection.subscribers.then_some(AudienceTarget::Subscribers))
        .chain(selection.named.iter().copied().map(AudienceTarget::Named))
        .collect()
}

/// Resolves an optional picker selection to the targets to persist. An absent
/// selection defaults to `[Public]` — the safe default for non-editor callers
/// that omit the field on the wire.
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
/// An explicit Private target and an empty target set both yield no selections.
#[must_use]
pub fn targets_to_audience_selection(targets: &[AudienceTarget]) -> AudienceSelection {
    let mut selection = AudienceSelection::default();
    for target in targets {
        match target {
            AudienceTarget::Public => selection.public = true,
            AudienceTarget::Subscribers => selection.subscribers = true,
            AudienceTarget::Named(id) => selection.named.push(*id),
            AudienceTarget::Private => {}
        }
    }
    selection
}

/// Admission seam: decides the initial status of a new subscription. Layer A
/// auto-approves (`Active`); M13 swaps the one impl below for an approval gate.
pub trait SubscriptionPolicy: Send + Sync {
    fn initial_status(
        &self,
        author_user_id: UserId,
        subscriber: &SubscriberIdentity,
    ) -> SubscriptionStatus;
}

/// Layer A NOOP policy: every subscription is admitted as `Active`.
pub struct OpenSubscriptionPolicy;

impl SubscriptionPolicy for OpenSubscriptionPolicy {
    fn initial_status(&self, _a: UserId, _s: &SubscriberIdentity) -> SubscriptionStatus {
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
    fn default_audience_round_trips_through_closed_enum_interfaces() {
        for (audience, token) in [
            (DefaultAudience::Public, "public"),
            (DefaultAudience::Subscribers, "subscribers"),
            (DefaultAudience::Private, "private"),
        ] {
            assert_eq!(audience.as_ref(), token);
            assert_eq!(audience.to_string(), token);
            assert_eq!(DefaultAudience::try_from(token), Ok(audience));

            let json = serde_json::to_string(&audience).unwrap();
            assert_eq!(json, format!("\"{token}\""));
            assert_eq!(
                serde_json::from_str::<DefaultAudience>(&json).unwrap(),
                audience
            );
        }
    }

    #[test]
    fn default_audience_rejects_unknown_and_whitespace_padded_tokens() {
        for token in ["unknown", " public", "public ", "\tpublic"] {
            assert!(
                DefaultAudience::try_from(token).is_err(),
                "{token:?} must reject"
            );
            assert!(
                serde_json::from_str::<DefaultAudience>(&format!("\"{token}\"")).is_err(),
                "{token:?} must not deserialize"
            );
        }

        let err: InvalidDefaultAudience = "unknown".parse::<DefaultAudience>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "default audience must be \"public\", \"subscribers\", or \"private\""
        );
    }

    #[test]
    fn default_audience_widens_to_its_matching_post_target() {
        for (default, target) in [
            (DefaultAudience::Public, AudienceTarget::Public),
            (DefaultAudience::Subscribers, AudienceTarget::Subscribers),
            (DefaultAudience::Private, AudienceTarget::Private),
        ] {
            assert_eq!(AudienceTarget::from(default), target);
        }
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
    fn open_policy_auto_approves() {
        assert_eq!(
            OpenSubscriptionPolicy.initial_status(
                UserId::from(1),
                &local_subscriber_identity(ChannelId::from(1), UserId::from(2))
            ),
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
    fn subscriber_ref_rejects_empty_and_unicode_whitespace() {
        for blank in ["", "\u{2003}", "\u{00a0}\t\n"] {
            assert!(
                blank.parse::<SubscriberRef>().is_err(),
                "expected {blank:?} to be rejected"
            );
        }
    }

    #[test]
    fn invalid_subscriber_ref_display_is_stable() {
        assert_eq!(
            InvalidSubscriberRef.to_string(),
            "subscriber reference must not be blank"
        );
    }

    #[test]
    fn subscriber_ref_serde_rejects_unicode_blank_input() {
        assert!(serde_json::from_str::<SubscriberRef>("\"\\u2003\"").is_err());
    }

    #[test]
    fn subscriber_ref_preserves_opaque_input_byte_for_byte() {
        let input = " \u{2003}HTTPS://Remote.Example/Alice%2F\u{00df}\n";
        let subscriber_ref = input
            .parse::<SubscriberRef>()
            .expect("fixture is a non-blank subscriber reference");
        assert_eq!(subscriber_ref.as_bytes(), input.as_bytes());
        assert_eq!(
            String::from(subscriber_ref.clone()).as_bytes(),
            input.as_bytes()
        );
        assert_eq!(
            serde_json::from_str::<SubscriberRef>(&serde_json::to_string(&subscriber_ref).unwrap())
                .unwrap()
                .as_bytes(),
            input.as_bytes()
        );
    }

    #[test]
    fn viewer_user_id_is_none_for_a_remote_viewer_with_a_numeric_ref() {
        // The #6 hole in its second form: a remote ref that happens to be the
        // decimal form of a local user id must not project to that user, or the
        // owner-only controls render for a viewer who is not the owner.
        let impostor = ViewerIdentity::Remote {
            channel_id: ChannelId::from(2),
            subscriber_ref: "42".parse().unwrap(),
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
                subscriber_ref: "https://remote.example/users/alice".parse().unwrap(),
            }),
            None
        );
    }

    fn selection(public: bool, subscribers: bool, named: &[AudienceId]) -> AudienceSelection {
        AudienceSelection {
            public,
            subscribers,
            named: named.to_vec(),
        }
    }

    #[test]
    fn selected_targets_union_without_losing_dominated_targets() {
        let selected = selection(true, true, &[AudienceId::from(5), AudienceId::from(9)]);
        let targets = vec![
            AudienceTarget::Public,
            AudienceTarget::Subscribers,
            AudienceTarget::Named(AudienceId::from(5)),
            AudienceTarget::Named(AudienceId::from(9)),
        ];
        assert_eq!(audience_selection_to_targets(&selected), targets);
        assert_eq!(targets_to_audience_selection(&targets), selected);
    }

    #[test]
    fn named_only_and_private_round_trip_without_a_built_in_target() {
        let named = selection(false, false, &[AudienceId::from(5)]);
        assert_eq!(
            audience_selection_to_targets(&named),
            vec![AudienceTarget::Named(AudienceId::from(5))]
        );
        assert_eq!(
            targets_to_audience_selection(&audience_selection_to_targets(&named)),
            named
        );
        assert!(audience_selection_to_targets(&AudienceSelection::default()).is_empty());
        assert_eq!(
            targets_to_audience_selection(&[]),
            AudienceSelection::default()
        );
        assert_eq!(
            targets_to_audience_selection(&[AudienceTarget::Private]),
            AudienceSelection::default()
        );
    }

    #[test]
    fn absent_web_selection_still_defaults_to_public_but_explicit_empty_is_private() {
        assert_eq!(
            audience_targets_or_public(None),
            vec![AudienceTarget::Public]
        );
        assert!(audience_targets_or_public(Some(&AudienceSelection::default())).is_empty());
    }

    #[test]
    fn obsolete_mutually_exclusive_wire_shape_is_rejected_not_treated_as_private() {
        assert!(
            serde_json::from_str::<AudienceSelection>(r#"{"base":"public","named":[]}"#).is_err()
        );
    }
}
