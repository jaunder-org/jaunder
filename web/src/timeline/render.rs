//! The timeline vertical's pure, projector-coincident render twin (ADR-0070's
//! extra leaf beside `state`/`component`): non-reactive markup only, so it stays
//! host-tested and coverage-measured.

use common::root_relative_url::RootRelativeUrl;
use common::seed::TimelineOrder;
use maud::html;

use crate::html::Markup;
use crate::icon::{self, Icons};

/// Builds the canonical URL for a timeline order. The route base must not carry
/// a query: newest is the bare base and oldest is the sole `order` parameter.
#[must_use]
pub fn order_url(base: &RootRelativeUrl, order: TimelineOrder) -> RootRelativeUrl {
    match order {
        TimelineOrder::Newest => base.clone(),
        TimelineOrder::Oldest => {
            let Ok(url) = RootRelativeUrl::try_from(format!("{base}?order=oldest")) else {
                unreachable!("a root-relative route base remains valid with the order query");
            };
            url
        }
    }
}

/// Returns the only order different from `order`.
#[must_use]
pub(super) const fn opposite_order(order: TimelineOrder) -> TimelineOrder {
    match order {
        TimelineOrder::Newest => TimelineOrder::Oldest,
        TimelineOrder::Oldest => TimelineOrder::Newest,
    }
}

/// Pure, projector-coincident timeline-order toggle. Reactive pages inject
/// these exact bytes and delegate its click event; this leaf owns its markup.
#[must_use]
pub(crate) fn order_control(order: TimelineOrder) -> Markup {
    let (icon_path, current) = match order {
        TimelineOrder::Newest => (Icons::SORT_DESCENDING, "newest"),
        TimelineOrder::Oldest => (Icons::SORT_ASCENDING, "oldest"),
    };
    let action = match opposite_order(order) {
        TimelineOrder::Newest => "Oldest first; show newest first",
        TimelineOrder::Oldest => "Newest first; show oldest first",
    };
    Markup::new(html! {
        div class="j-timeline-order" data-jaunder-part="timeline-order" {
            button
                type="button"
                class="j-icon-btn j-timeline-order-button"
                data-order=(current)
                aria-label=(action)
                title=(action)
            {
                (icon::render(icon_path, 18))
            }
        }
    })
}

/// The non-functional "Load more" button the projector paints so the reactive
/// button (which replaces it on boot) doesn't reflow. Rendered only when there is
/// a next page, matching the reactive `has_more` guard.
#[must_use]
pub(crate) fn load_more(has_more: bool) -> Markup {
    Markup::new(html! {
        @if has_more {
            button data-jaunder-part="continuation" { "Load more" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{load_more, opposite_order, order_control, order_url};
    use common::seed::TimelineOrder;

    fn route(path: &str) -> common::root_relative_url::RootRelativeUrl {
        path.parse().expect("test route is root-relative")
    }

    #[test]
    fn opposite_order_toggles_both_directions() {
        assert_eq!(opposite_order(TimelineOrder::Newest), TimelineOrder::Oldest);
        assert_eq!(opposite_order(TimelineOrder::Oldest), TimelineOrder::Newest);
    }

    #[test]
    fn order_control_exposes_current_direction_and_toggle_action() {
        let control = order_control(TimelineOrder::Oldest).into_string();
        assert!(control.contains("data-order=\"oldest\""), "{control}");
        assert!(
            control.contains("aria-label=\"Oldest first; show newest first\""),
            "{control}"
        );
        assert!(
            control.contains(crate::icon::Icons::SORT_ASCENDING),
            "{control}"
        );
    }

    #[test]
    fn order_urls_are_canonical_for_every_timeline_route_shape() {
        for base in ["/", "/app", "/~alice", "/tags/rust", "/~alice/tags/rust"] {
            let base = route(base);
            assert_eq!(order_url(&base, TimelineOrder::Newest), base);
            let oldest = order_url(&base, TimelineOrder::Oldest);
            let oldest: &str = oldest.as_ref();
            assert_eq!(oldest, format!("{base}?order=oldest"));
        }
    }

    #[test]
    fn load_more_placeholder_renders_when_more_rows_exist() {
        assert_eq!(
            load_more(true),
            "<button data-jaunder-part=\"continuation\">Load more</button>"
        );
    }

    #[test]
    fn load_more_placeholder_renders_empty_without_next_page() {
        assert_eq!(load_more(false), "");
    }
}
