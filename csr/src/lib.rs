#![cfg(target_arch = "wasm32")]
// web::app::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors web/src/lib.rs).
#![recursion_limit = "512"]

use client::perf::{BOOT_ENTRY, BOOT_MOUNT_DONE, BOOT_RENDER_START, BOOT_SEED_PARSED, mark};
use common::seed::PageSeed;
use leptos::prelude::*;
use web::app::App;

// The e2e suite waits on `body[data-mounted]` as the "app is mounted and
// interactive" signal — the counterpart of `MOUNTED_ATTR` in
// `end2end/tests/mount.ts`. The two literals must agree; if they drift, every
// e2e test times out. That agreement is enforced by the `xlang-literal` gate
// (`xtask/src/steps/xlang_literal_check.rs`), which reads this literal and its
// TypeScript counterpart and fails `cargo xtask check` when they differ (#767).
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_ready() {
        if (document && document.body) {
            document.body.setAttribute('data-mounted', 'true');
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
    mark(BOOT_SEED_PARSED);
    // App re-renders the identical content from `seed`, so removing the
    // server-painted copy avoids a duplicate paint without a visible flash (the
    // removal and remount happen in one synchronous task).
    client::dom::remove_element_by_id("app");
    // Drop the projector-painted discovery <link>s so the reactive FeedDiscovery/
    // RsdDiscovery mounted below produce the ONLY set (no invisible duplicate). Crawlers/
    // no-JS never run this, so their head is unchanged (#198).
    client::dom::remove_elements_by_selector(&format!("link[{}]", web::app::DISCOVERY_MARKER_ATTR));
    mark(BOOT_RENDER_START);
    leptos::mount::mount_to_body(move || {
        provide_context(seed.clone());
        view! { <App /> }
    });
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    // First statement: `BOOT_ENTRY` is the wasm's own "I am running" timestamp, and
    // the harness derives fetch/compile/instantiate from the gap before it.
    mark(BOOT_ENTRY);
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    mount();
    mark(BOOT_MOUNT_DONE);
    mark_ready();
}
