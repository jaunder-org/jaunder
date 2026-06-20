//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use std::fmt;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }
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
}

str_enum!(Channel { Local => "local" });
str_enum!(SubscriptionStatus { Active => "active", Pending => "pending", Blocked => "blocked" });
str_enum!(TargetKind { Public => "public", Subscribers => "subscribers", Named => "named" });

/// Who is reading. Wider than Layer A needs (only `Anonymous` and the local
/// channel are constructed today) so non-local channels need no signature change
/// in Layers B/C. `subscriber_ref` makes this non-`Copy`. See ADR-0020.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerIdentity {
    Anonymous,
    Channel {
        channel_id: i64,
        subscriber_ref: String,
    },
}

impl ViewerIdentity {
    /// Local viewer constructor used by Layer A: a logged-in account on the
    /// `local` channel, keyed by its user id as the `subscriber_ref`.
    #[must_use]
    pub fn local(user_id: i64, local_channel_id: i64) -> Self {
        Self::Channel {
            channel_id: local_channel_id,
            subscriber_ref: user_id.to_string(),
        }
    }
}

/// What a post is addressed to, as chosen in the editor / API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudienceTarget {
    Public,
    Private,
    Subscribers,
    Named(i64),
}

/// Admission seam: decides the initial status of a new subscription. Layer A
/// auto-approves (`Active`); M13 swaps the one impl below for an approval gate.
pub trait SubscriptionPolicy: Send + Sync {
    fn initial_status(
        &self,
        author_user_id: i64,
        channel_id: i64,
        subscriber_ref: &str,
    ) -> SubscriptionStatus;
}

/// Layer A NOOP policy: every subscription is admitted as `Active`.
pub struct OpenSubscriptionPolicy;

impl SubscriptionPolicy for OpenSubscriptionPolicy {
    fn initial_status(&self, _a: i64, _c: i64, _r: &str) -> SubscriptionStatus {
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
    }

    #[test]
    fn open_policy_returns_active() {
        assert_eq!(
            OpenSubscriptionPolicy.initial_status(1, 1, "1"),
            SubscriptionStatus::Active
        );
    }

    #[test]
    fn viewer_local_constructor_uses_user_id_as_subscriber_ref() {
        let viewer = ViewerIdentity::local(42, 7);
        assert_eq!(
            viewer,
            ViewerIdentity::Channel {
                channel_id: 7,
                subscriber_ref: "42".to_string(),
            }
        );
    }
}
