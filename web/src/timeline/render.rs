//! The timeline vertical's pure, projector-coincident render twin (ADR-0070's
//! extra leaf beside `state`/`component`): plain-string building only, so it stays
//! host-tested and coverage-measured.

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

#[cfg(test)]
mod tests {
    use super::render_load_more;

    #[test]
    fn load_more_placeholder_renders_when_more_rows_exist() {
        assert_eq!(render_load_more(true), "<button>Load more</button>");
    }

    #[test]
    fn load_more_placeholder_renders_empty_without_next_page() {
        assert_eq!(render_load_more(false), "");
    }
}
