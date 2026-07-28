//! Shared pure, non-reactive HTML leaf primitives for the web crate.
//!
//! Plain-string building only — no leptos reactivity, like `common::feed` — so
//! `reactive_graph` never sits on the public request path (the #173 escape). These
//! leaves are shared across verticals: HTML escaping, SVG icon path data, tag-link
//! context, the home hero/masthead, the projector "Load more" placeholder, and a
//! byte-size formatter. Several are the pure twins the projector
//! (`server::projector`, via `crate::posts::render`) and the reactive components
//! both render, so their output coincides byte-for-byte (ADR-0041). See
//! `docs/adr/0041` and `docs/inbound-data-handling.md` §4.
//!
//! The page-frame shell projector (`render_shell`/`render_head` + the shell
//! constants) moved to `crate::app::render` with the reactive shell it twins
//! (#330); the remaining primitives here are dissolved onto their co-located homes
//! by #658.

/// The non-functional "Load more" button the projector paints so the reactive
/// button (which replaces it on boot) doesn't reflow. Rendered only when there is
/// a next page, matching the reactive `has_more` guard.
#[must_use]
pub(crate) fn render_load_more(has_more: bool) -> String {
    if has_more {
        "<button>Load more</button>".to_string()
    } else {
        String::new()
    }
}

/// Formats a byte count as a human-readable size (`B` / `KB` / `MB` / `GB`, one
/// decimal). Shared display formatter, host-tested here.
#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts < 2^52 convert to f64 exactly; larger values only affect a \
              human-readable one-decimal display, so any loss is immaterial"
)]
pub fn format_bytes(bytes: impl Into<i64>) -> String {
    const KB: i64 = 1_024;
    const MB: i64 = 1_024 * KB;
    const GB: i64 = 1_024 * MB;

    // Generic over the byte-ish newtypes (`ByteSize`, `MaxFileSize`, `UserQuota` — each
    // `Into<i64>` via `NumNewtype`) as well as a bare `i64`, so call sites pass the typed
    // value without spelling `.value()`.
    let bytes: i64 = bytes.into();

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }
}
