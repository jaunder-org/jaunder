//! Subscribe / unsubscribe `#[server]` functions for the `local` channel.
//!
//! These power the Subscribe / Unsubscribe button on a user's profile
//! (timeline) page. Layer A only supports *local* subscriptions: the viewer
//! must be a logged-in local account, and the subscription is recorded on the
//! seeded `local` channel keyed by the viewer's `user_id` (rendered as the
//! `subscriber_ref`). Creation routes through the store's admission seam
//! (`OpenSubscriptionPolicy` → `active` in Layer A; an approval gate later).
//!
//! Self-subscription is rejected — an author cannot subscribe to themselves.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(feature = "server")]
mod server;
pub mod state;

pub use api::{IsSubscribed, Subscribe, Unsubscribe, is_subscribed, subscribe, unsubscribe};
#[cfg(target_arch = "wasm32")]
pub use component::SubscribeButton;
pub use state::{SubscribePaint, paint};
