//! The timeline vertical's pure, projector-coincident render twin (ADR-0070's
//! extra leaf beside `state`/`component`): non-reactive markup only, so it stays
//! host-tested and coverage-measured.

use common::root_relative_url::RootRelativeUrl;
use common::seed::TimelineOrder;
use maud::html;

use crate::html::Markup;

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

/// Pure, projector-coincident timeline-order control. Reactive pages inject
/// these exact bytes and delegate its change event; this leaf owns its markup.
#[must_use]
pub(crate) fn order_control(order: TimelineOrder) -> Markup {
    Markup::new(html! {
        div data-jaunder-part="timeline-order" {
            label for="timeline-order-select" { "Order" }
            select id="timeline-order-select" {
                option value="newest" selected[order == TimelineOrder::Newest] { "Newest" }
                option value="oldest" selected[order == TimelineOrder::Oldest] { "Oldest" }
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
    use super::{load_more, order_control, order_url};
    use common::seed::TimelineOrder;

    fn route(path: &str) -> common::root_relative_url::RootRelativeUrl {
        path.parse().expect("test route is root-relative")
    }

    #[test]
    fn order_control_marks_exactly_the_active_option() {
        assert_eq!(
            order_control(TimelineOrder::Oldest),
            "<div data-jaunder-part=\"timeline-order\"><label for=\"timeline-order-select\">Order</label><select id=\"timeline-order-select\"><option value=\"newest\">Newest</option><option value=\"oldest\" selected>Oldest</option></select></div>"
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
