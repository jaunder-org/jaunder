use crate::auth::require_auth;
use crate::error::{InternalError, InternalResult};
use leptos::prelude::*;
use std::sync::Arc;
use storage::UserStorage;

pub async fn require_operator() -> InternalResult<()> {
    let auth = require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let Some(user) = users
        .get_user(auth.user_id)
        .await
        .map_err(InternalError::storage)?
    else {
        return Err(InternalError::unauthorized("user does not exist")); // cov:ignore
    };

    if !user.is_operator {
        return Err(InternalError::unauthorized("operator access required"));
    }

    Ok(())
}
