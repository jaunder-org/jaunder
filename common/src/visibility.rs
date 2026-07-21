//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use crate::ids::{AudienceId, ChannelId, UserId};
use macros::StrEnum;

// String-backed enums ride the `StrEnum` trailer: `as_str`/`Display`/`FromStr`/
// `TryFrom<&str>` + a generated `Invalid<Name>` error, with the wire token defaulting to the
// lowercased variant name. Wire-facing enums add `#[str_enum(serde)]`; std derives (incl.
// `Default` via `#[default]`) stay in each enum's own list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum Channel {
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum SubscriptionStatus {
    Active,
    Pending,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum TargetKind {
    Public,
    Subscribers,
    Named,
}

// The mutually-exclusive built-in audience base chosen in the editor / API — the
// typed form of the audience-picker's `base`. Composes with named audiences by
// union except for `Private` (author-only), which is the safe, non-widening
// `Default` (faithful to the prior empty-string -> author-only fall-through). #499.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, StrEnum)]
#[str_enum(serde)]
pub enum AudienceBase {
    #[default]
    Private,
    Public,
    Subscribers,
}

/// Who is reading. Wider than Layer A needs (only `Anonymous` and the local
/// channel are constructed today) so non-local channels need no signature change
/// in Layers B/C. `subscriber_ref` makes this non-`Copy`. See ADR-0020.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerIdentity {
    Anonymous,
    Channel {
        channel_id: ChannelId,
        subscriber_ref: String,
    },
}

impl ViewerIdentity {
    /// Local viewer constructor used by Layer A: a logged-in account on the
    /// `local` channel, keyed by its user id as the `subscriber_ref`.
    #[must_use]
    pub fn local(user_id: UserId, local_channel_id: ChannelId) -> Self {
        Self::Channel {
            channel_id: local_channel_id,
            subscriber_ref: user_id.to_string(),
        }
    }
}

/// Projects an authenticated account plus the resolved `local` channel id into a
/// [`ViewerIdentity`].
///
/// `Some(channel_id)` → a `local` channel viewer; `None` (the `local` channel id
/// could not be resolved) → [`ViewerIdentity::Anonymous`], fail-closed: a viewer
/// we cannot positively place on a channel gets no non-public reach.
#[must_use]
pub fn account_viewer(user_id: UserId, local_channel_id: Option<ChannelId>) -> ViewerIdentity {
    match local_channel_id {
        Some(channel_id) => ViewerIdentity::local(user_id, channel_id),
        None => ViewerIdentity::Anonymous,
    }
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
        ViewerIdentity::Channel { subscriber_ref, .. } => subscriber_ref.parse::<UserId>().ok(),
        ViewerIdentity::Anonymous => None,
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
            assert_eq!(TargetKind::try_from(k.as_str()), Ok(k));
        }
        assert!(TargetKind::try_from("private").is_err());
    }

    #[test]
    fn display_matches_as_str() {
        // Covers the macro-generated `Display` impl for every enum, including
        // the `SubscriptionStatus` variants reserved for later milestones that
        // have no lookup row (and thus no bijection-test exposure) yet.
        assert_eq!(Channel::Local.to_string(), Channel::Local.as_str());
        for s in [
            SubscriptionStatus::Active,
            SubscriptionStatus::Pending,
            SubscriptionStatus::Blocked,
        ] {
            assert_eq!(s.to_string(), s.as_str());
            assert_eq!(SubscriptionStatus::try_from(s.as_str()), Ok(s));
        }
        for k in [
            TargetKind::Public,
            TargetKind::Subscribers,
            TargetKind::Named,
        ] {
            assert_eq!(k.to_string(), k.as_str());
        }
        for b in [
            AudienceBase::Public,
            AudienceBase::Subscribers,
            AudienceBase::Private,
        ] {
            assert_eq!(b.to_string(), b.as_str());
            assert_eq!(AudienceBase::try_from(b.as_str()), Ok(b));
        }
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
    fn viewer_local_constructor_uses_user_id_as_subscriber_ref() {
        let viewer = ViewerIdentity::local(UserId::from(42), ChannelId::from(7));
        assert_eq!(
            viewer,
            ViewerIdentity::Channel {
                channel_id: ChannelId::from(7),
                subscriber_ref: "42".to_string(),
            }
        );
    }

    #[test]
    fn account_viewer_with_channel_is_local() {
        assert_eq!(
            account_viewer(UserId::from(7), Some(ChannelId::from(3))),
            ViewerIdentity::local(UserId::from(7), ChannelId::from(3)),
            "a resolved local channel yields a Channel viewer keyed by the user id",
        );
    }

    #[test]
    fn account_viewer_without_channel_fails_closed_to_anonymous() {
        assert_eq!(
            account_viewer(UserId::from(7), None),
            ViewerIdentity::Anonymous,
            "an unresolved local channel must fail closed to Anonymous",
        );
    }

    #[test]
    fn viewer_user_id_projects_local_channel_to_user_id() {
        assert_eq!(
            viewer_user_id(&ViewerIdentity::local(UserId::from(42), ChannelId::from(1))),
            Some(UserId::from(42))
        );
    }

    #[test]
    fn viewer_user_id_is_none_for_anonymous() {
        assert_eq!(viewer_user_id(&ViewerIdentity::Anonymous), None);
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
        assert!(audience_selection_to_targets(&selection(
            AudienceBase::Private,
            &[AudienceId::from(5)]
        ))
        .is_empty());
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
