//! Named-audience management for the account area: the `#[server]` functions and
//! the co-located reactive UI (`AudiencesPage` and its child components).
//!
//! These let an author curate named groups of their own active subscribers and
//! assign/unassign subscribers to those groups. They back the Audiences screen
//! under the account/settings nav and feed the post-editor audience picker.
//!
//! ## Authorization
//!
//! Every function derives `author_user_id` from the authenticated session
//! ([`require_auth`]) — **never** from a client parameter. Every store method is
//! author-scoped (it takes `author_user_id` and filters by it), so passing the
//! session's `user_id` is the whole authorization: a client-supplied
//! `audience_id` owned by another author matches nothing (an empty list, or a
//! no-op delete).

mod api;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(feature = "server")]
mod server;

pub use api::{
    add_subscriber_to_audience, create_audience, delete_audience, list_audience_members,
    list_my_audiences, list_my_subscribers, remove_subscriber_from_audience, rename_audience,
    AddSubscriberToAudience, AudienceSummary, CreateAudience, DeleteAudience, ListAudienceMembers,
    ListMyAudiences, ListMySubscribers, RemoveSubscriberFromAudience, RenameAudience,
    SubscriberSummary,
};
#[cfg(target_arch = "wasm32")]
pub use component::AudiencesPage;
