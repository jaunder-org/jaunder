//! What the subscribe control paints, decided away from the component.
//!
//! The component cannot be host-tested (it is wasm-only, ADR-0070) and is already at
//! the `thin-components` setup budget, so the decision lives here where it can be
//! asserted directly — the `timeline::state` shape (#306).

use common::username::Username;

use crate::error::{WebError, WebResult};

/// What the subscribe control shows once its query resolves.
///
/// Failure **is** a variant here, unlike [`crate::timeline::state::TimelinePaint`],
/// because this control's failure mode is the point: a subscription check that
/// errors must not be paintable as an answer about the subscription. See
/// [`paint`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscribePaint {
    /// Nothing to show: no viewer, or the viewer is looking at their own profile.
    Hidden,
    /// The viewer's subscription state is known — `true` renders Unsubscribe.
    Toggle(bool),
    /// The subscription state could not be determined.
    Failed(WebError),
}

/// Decides the paint from the viewer, the profile being viewed, and the query result.
///
/// **The viewer is checked before the result**, which is load-bearing rather than
/// stylistic: `is_subscribed` returns `Err(Unauthorized)` for a logged-out viewer, and
/// a logged-out viewer must see nothing at all. Reading the result first would paint an
/// error on every anonymous profile view.
///
/// Beyond that, an `Err` is [`SubscribePaint::Failed`] and never
/// `Toggle(false)` (#861). Collapsing the two makes "Subscribe is visible" mean
/// "you are not subscribed, **or** we could not find out" — an e2e helper waiting
/// on that button as proof of an unsubscribe, and a user whose check failed being
/// told they have no subscription they in fact have.
#[must_use]
pub fn paint(
    viewer: Option<&Username>,
    profile: &Username,
    subscribed: WebResult<bool>,
) -> SubscribePaint {
    match viewer {
        Some(name) if name != profile => match subscribed {
            Ok(subscribed) => SubscribePaint::Toggle(subscribed),
            Err(err) => SubscribePaint::Failed(err),
        },
        _ => SubscribePaint::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_username;

    fn viewer() -> Username {
        parse_username("bob")
    }

    fn author() -> Username {
        parse_username("alice")
    }

    #[test]
    fn a_resolved_subscription_paints_the_toggle() {
        assert_eq!(
            paint(Some(&viewer()), &author(), Ok(true)),
            SubscribePaint::Toggle(true)
        );
        assert_eq!(
            paint(Some(&viewer()), &author(), Ok(false)),
            SubscribePaint::Toggle(false)
        );
    }

    /// The #861 regression: a failed check must not be paintable as "not subscribed".
    #[test]
    fn a_failed_check_paints_failed_not_an_unsubscribed_toggle() {
        let err = WebError::Storage {
            message: "database is locked".to_owned(),
        };
        let painted = paint(Some(&viewer()), &author(), Err(err.clone()));

        assert_eq!(painted, SubscribePaint::Failed(err));
        assert_ne!(painted, SubscribePaint::Toggle(false));
    }

    #[test]
    fn own_profile_is_hidden_whatever_the_check_said() {
        assert_eq!(
            paint(Some(&author()), &author(), Ok(true)),
            SubscribePaint::Hidden
        );
        assert_eq!(
            paint(Some(&author()), &author(), Ok(false)),
            SubscribePaint::Hidden
        );
    }

    /// A logged-out viewer gets `Err(Unauthorized)` from `is_subscribed`, and must
    /// still see nothing — not an error. This is why `paint` reads the viewer first.
    #[test]
    fn a_logged_out_viewer_is_hidden_not_failed() {
        assert_eq!(
            paint(None, &author(), Err(WebError::Unauthorized)),
            SubscribePaint::Hidden
        );
    }

    /// Own profile *and* an error: still nothing, for the same reason.
    #[test]
    fn own_profile_with_a_failed_check_is_hidden() {
        assert_eq!(
            paint(
                Some(&author()),
                &author(),
                Err(WebError::Storage {
                    message: "boom".to_owned()
                })
            ),
            SubscribePaint::Hidden
        );
    }
}
