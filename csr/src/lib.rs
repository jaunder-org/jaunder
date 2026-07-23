#![cfg(target_arch = "wasm32")]
// web::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors web/src/lib.rs).
#![recursion_limit = "512"]

use leptos::prelude::*;
use web::render::PageSeed;
use web::App;

// The e2e suite waits on `body[data-hydrated]` (end2end/tests/hydration.ts) as the
// "app is mounted and interactive" signal. CSR has no hydration, but the same marker
// cleanly means "mount_to_body done" here, so the specs need no changes.
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_ready() {
        if (document && document.body) {
            document.body.setAttribute('data-hydrated', 'true');
        }
    }
")]
extern "C" {
    fn mark_ready();
}

/// Boot the CSR client (#179). Adopts the public projector's data blob (#178):
/// reads `#jaunder-seed`, drops the projector-painted `#app` container, and mounts
/// [`App`] with the seed in context so the public pages render their first paint from
/// it (no reactive fetch) via the same `render` fn the projector used — coincident,
/// flash-free. On the static SPA shell (no blob, no `#app`) the seed is `None` and
/// this is an ordinary `mount_to_body`.
fn mount() {
    let seed = client::dom::text_content_by_id("jaunder-seed")
        .and_then(|json| serde_json::from_str::<PageSeed>(&json).ok());
    // App re-renders the identical content from `seed`, so removing the
    // server-painted copy avoids a duplicate paint without a visible flash (the
    // removal and remount happen in one synchronous task).
    client::dom::remove_element_by_id("app");
    // Drop the projector-painted discovery <link>s so the reactive FeedDiscovery/
    // RsdDiscovery mounted below produce the ONLY set (no invisible duplicate). Crawlers/
    // no-JS never run this, so their head is unchanged (#198).
    client::dom::remove_elements_by_selector(&format!(
        "link[{}]",
        web::render::DISCOVERY_MARKER_ATTR
    ));
    leptos::mount::mount_to_body(move || {
        provide_context(seed.clone());
        view! { <App /> }
    });
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    mount();
    mark_ready();
}
