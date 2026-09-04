//! Typed wire API for the operator `WebSub` recovery surface.

use crate::error::WebResult;
use common::MutationOutcome;
use common::ids::FeedEventId;
use common::pagination::PageSize;
use common::tagged_url::HubUrl;
use common::time::UtcInstant;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use super::server::{
    get_settings_impl, list_dead_letters_impl, redrive_dead_letters_impl, update_hub_impl,
};

/// The configured publisher hub shown on the settings card.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WebsubSettings {
    pub hub_url: Option<HubUrl>,
}

/// Failed stage selected by an operator. The two states deliberately mirror the
/// worker's durable phase names without exposing the storage enum on the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebsubPhase {
    Regeneration,
    Publication,
}

/// Opaque stable newest-first keyset position.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeadLetterCursor {
    pub terminal_at: UtcInstant,
    pub id: FeedEventId,
}

/// Operator-safe terminal-event projection. Diagnostics cross this boundary only
/// after operator authorization has succeeded.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeadLetterRow {
    pub id: FeedEventId,
    pub feed_path: String,
    pub phase: WebsubPhase,
    pub attempts: i32,
    pub terminal_at: UtcInstant,
    pub diagnostic: Option<String>,
}

/// One bounded dead-letter page and its successor cursor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeadLetterPage {
    pub events: Vec<DeadLetterRow>,
    pub next_cursor: Option<DeadLetterCursor>,
}

#[macros::server(skip_all)]
pub async fn get_websub_settings() -> WebResult<WebsubSettings> {
    get_settings_impl().await
}

#[macros::server(skip_all)]
pub async fn update_websub_hub(hub_url: Option<HubUrl>) -> WebResult<MutationOutcome<()>> {
    update_hub_impl(hub_url).await
}

#[macros::server(skip_all)]
pub async fn list_dead_letters(
    phase: WebsubPhase,
    cursor: Option<DeadLetterCursor>,
    page_size: PageSize,
) -> WebResult<DeadLetterPage> {
    list_dead_letters_impl(phase, cursor, page_size).await
}

#[macros::server(skip_all)]
pub async fn redrive_dead_letters(ids: Vec<FeedEventId>) -> WebResult<MutationOutcome<()>> {
    redrive_dead_letters_impl(ids).await
}
