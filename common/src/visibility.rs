//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use crate::ids::{AudienceId, ChannelId, UserId};
use std::fmt;

macro_rules! str_enum {
    // Internal: the inherent `as_str`, `TryFrom<&str>`, and `Display` impls,
    // shared by every public arm so the enum-definition and the impls stay in
    // one place.
    (@impls $name:ident { $($variant:ident => $s:literal),+ }) => {
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $s),+ }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = ();
            fn try_from(s: &str) -> Result<Self, ()> {
                match s { $($s => Ok(Self::$variant),)+ _ => Err(()) }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
    };
    // Internal: wire `Serialize`/`Deserialize` routed through `as_str`/`TryFrom`,
    // so the invocation's string literals are the single source of truth for the
    // wire form (no `rename_all` drift).
    (@serde $name:ident { $($s:literal),+ }) => {
        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> ::core::result::Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::core::result::Result<Self, D::Error> {
                // Fully-qualify: the `Deserialize` trait is not `use`d in this
                // module, so a bare `String::deserialize` method path would not
                // resolve at the macro-expansion site.
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(d)?;
                Self::try_from(s.as_str())
                    .map_err(|()| <D::Error as ::serde::de::Error>::unknown_variant(&s, &[$($s),+]))
            }
        }
    };
    // Plain string enum: DB/lookup facing, no serde.
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }
        str_enum!(@impls $name { $($variant => $s),+ });
    };
    // Wire-facing string enum: adds serde. The variant tagged `default` becomes
    // the derived `Default` (kept first so the `#[default]` attaches to it).
    (serde $name:ident { default $dvar:ident => $ds:literal $(, $variant:ident => $s:literal)* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
        pub enum $name {
            #[default]
            $dvar,
            $($variant),*
        }
        str_enum!(@impls $name { $dvar => $ds $(, $variant => $s)* });
        str_enum!(@serde $name { $ds $(, $s)* });
    };
}

str_enum!(Channel { Local => "local" });
str_enum!(SubscriptionStatus { Active => "active", Pending => "pending", Blocked => "blocked" });
str_enum!(TargetKind { Public => "public", Subscribers => "subscribers", Named => "named" });

// The mutually-exclusive built-in audience base chosen in the editor / API — the
// typed form of the audience-picker's `base`. Composes with named audiences by
// union except for `Private` (author-only), which is the safe, non-widening
// `Default` (faithful to the prior empty-string -> author-only fall-through). #499.
str_enum!(serde AudienceBase { default Private => "private", Public => "public", Subscribers => "subscribers" });

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
}
