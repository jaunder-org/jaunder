//! Invitation-page authority projection shared by the wasm component and host tests.

use crate::error::WebResult;
use common::{registration::RegistrationPolicy, session_user::SessionUser};
use serde::{Deserialize, Serialize};

/// The CSR invitation page's presentation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PageAccess {
    Unavailable,
    Issuer { show_ledger: bool },
}

/// Projects the shared policy and reconciled viewer into the invitation page's CSR state.
#[must_use]
pub(crate) fn page_access(policy: RegistrationPolicy, viewer: Option<&SessionUser>) -> PageAccess {
    let Some(viewer) = viewer else {
        return PageAccess::Unavailable;
    };
    if !policy.may_issue_invitation(viewer.is_operator) {
        return PageAccess::Unavailable;
    }
    PageAccess::Issuer {
        show_ledger: policy.may_list_invitations(viewer.is_operator),
    }
}

/// Propagates the policy and session fetch results before projecting the CSR state.
pub(crate) fn resolve_page_access(
    policy: WebResult<RegistrationPolicy>,
    viewer: WebResult<Option<SessionUser>>,
) -> WebResult<PageAccess> {
    Ok(page_access(policy?, viewer?.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_username;

    fn viewer(is_operator: bool) -> SessionUser {
        SessionUser {
            username: parse_username("viewer"),
            is_operator,
        }
    }

    #[test]
    fn page_access_matches_each_policy_and_viewer_role() {
        let cases = [
            (RegistrationPolicy::Closed, false, PageAccess::Unavailable),
            (RegistrationPolicy::Closed, true, PageAccess::Unavailable),
            (
                RegistrationPolicy::OperatorInvites,
                false,
                PageAccess::Unavailable,
            ),
            (
                RegistrationPolicy::OperatorInvites,
                true,
                PageAccess::Issuer { show_ledger: true },
            ),
            (
                RegistrationPolicy::MemberInvites,
                false,
                PageAccess::Issuer { show_ledger: false },
            ),
            (
                RegistrationPolicy::MemberInvites,
                true,
                PageAccess::Issuer { show_ledger: true },
            ),
            (RegistrationPolicy::Open, false, PageAccess::Unavailable),
            (RegistrationPolicy::Open, true, PageAccess::Unavailable),
        ];

        for (policy, is_operator, expected) in cases {
            assert_eq!(page_access(policy, Some(&viewer(is_operator))), expected);
        }
        assert_eq!(
            page_access(RegistrationPolicy::MemberInvites, None),
            PageAccess::Unavailable
        );
    }

    #[test]
    fn resolver_projects_success_and_propagates_fetch_errors() {
        assert_eq!(
            resolve_page_access(
                Ok(RegistrationPolicy::MemberInvites),
                Ok(Some(viewer(false))),
            ),
            Ok(PageAccess::Issuer { show_ledger: false })
        );

        let policy_error = crate::error::WebError::validation("policy unavailable");
        assert_eq!(
            resolve_page_access(Err(policy_error.clone()), Ok(Some(viewer(true)))),
            Err(policy_error)
        );

        let session_error = crate::error::WebError::validation("session unavailable");
        assert_eq!(
            resolve_page_access(
                Ok(RegistrationPolicy::MemberInvites),
                Err(session_error.clone())
            ),
            Err(session_error)
        );
    }
}
