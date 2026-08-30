//! The linking context a tag chip is rendered under — the one knob that decides
//! whether a chip also links "here" (the author's own tag page).

use common::username::Username;

/// Linking context for a post's footer tag chips. `SiteWide` links each chip to
/// `/tags/:slug` only; `ForUser` also renders the "· here" link to
/// `/~:username/tags/:slug`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagCtx {
    SiteWide,
    ForUser(Username),
}
