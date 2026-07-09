use crate::auth::require_auth;
use crate::error::{InternalError, InternalResult};
use leptos::prelude::*;
use std::sync::Arc;
use storage::UserStorage;

pub async fn require_operator() -> InternalResult<()> {
    let auth = require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let Some(user) = users.get_user(auth.user_id).await? else {
        return Err(InternalError::unauthorized("user does not exist"));
    };

    if !user.is_operator {
        return Err(InternalError::unauthorized("operator access required"));
    }

    Ok(())
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::require_operator;
    use crate::error::WebError;
    use crate::test_support::auth_parts;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{MockUserStorage, UserStorage};

    // guard:no-backend — mock store
    #[tokio::test]
    async fn require_operator_rejects_when_user_absent() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(1, "ghost"));
        let mut users = MockUserStorage::new();
        users.expect_get_user().returning(|_uid| Ok(None));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);

        let result = require_operator().await;
        drop(owner);
        let err = result.unwrap_err();
        assert!(matches!(
            crate::error::project(err.kind(), err.public_message()),
            WebError::Unauthorized
        ));
    }
}
