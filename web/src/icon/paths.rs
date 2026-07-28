//! The glyph path data behind every Jaunder icon — the one source of truth the
//! reactive [`Icon`](super::Icon) component and the pure [`render`](super::render)
//! twin both draw from, so their `<svg>` output coincides (ADR-0041).

/// SVG path `d` attribute strings for all Jaunder icons. Shared by the reactive
/// [`crate::icon::Icon`] component and the pure [`crate::icon::render`].
pub struct Icons;

impl Icons {
    pub const HOME: &'static str = "M3 10l7-6 7 6v7a1 1 0 0 1-1 1h-4v-5H8v5H4a1 1 0 0 1-1-1z";
    pub const LOCAL: &'static str = "M4 5h12v10H4z M4 9h12";
    pub const FED: &'static str =
        "M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM3 10h14 M10 3c2 3 2 11 0 14 M10 3c-2 3-2 11 0 14";
    pub const REPLY: &'static str = "M4 4h12v9H7l-3 3z";
    pub const BOOKMARK: &'static str = "M5 3h10v14l-5-3-5 3z";
    pub const BOOST: &'static str =
        "M5 8l4-4 4 4 M4 7v4a3 3 0 0 0 3 3h9 M15 12l-4 4-4-4 M16 13V9a3 3 0 0 0-3-3H4";
    pub const HEART: &'static str =
        "M10 17s-7-4.5-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 17 7c0 5.5-7 10-7 10z";
    pub const SEARCH: &'static str = "M8 3a6 6 0 1 0 0 12a6 6 0 0 0 0-12z M17 17l-4-4";
    pub const PLUS: &'static str = "M10 4v12 M4 10h12";
    pub const COG: &'static str = "M10 6v2 M10 12v2 M6 10H4 M16 10h-2 M6.5 6.5l-1.5-1.5 M14 14l1.5 1.5 M6.5 13.5L5 15 M14 6l1.5-1.5 M10 13a3 3 0 1 0 0-6a3 3 0 0 0 0 6z";
    pub const EDIT: &'static str = "M3 17l4 0 9-9a2.83 2.83 0 0 0-4-4l-9 9 0 4 M12 5l3 3";
    pub const SHIELD: &'static str = "M10 3l6 2v4c0 4-2.4 7.1-6 8-3.6-.9-6-4-6-8V5l6-2z";
    pub const MEDIA: &'static str =
        "M3 5h14v10H3z M7 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2z M5 13l3-3 2 2 3-3 5 5H3z";
    pub const REFRESH: &'static str = "M15.5 8A6 6 0 1 0 16 11.5 M15.5 4v4h-4";
}
