#![cfg(target_arch = "wasm32")]
// web::app::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors web/src/lib.rs).
#![recursion_limit = "512"]

use client::{dom, perf, telemetry};
use common::client_telemetry::{ClientErrorContext, ClientSourceKind};
use common::seed::{PageSeed, PublicPresentation};

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

fn projector_seed() -> Option<PublicPresentation<PageSeed>> {
    let json = dom::text_content_by_id("jaunder-seed");
    let Ok(seed) = web::app::decode_projector_seed(json.as_deref()) else {
        let source_kind = ClientSourceKind::InvalidSeed;
        telemetry::report_swallowed(
            telemetry::error_kind(source_kind),
            ClientErrorContext::ProjectorSeedDecode,
            source_kind,
        );
        return None;
    };
    seed
}

/// Boot the CSR client (#179). Adopts the public projector's data blob (#178):
/// reads `#jaunder-seed`, drops the projector-painted `#app` container, and mounts
/// [`App`] with the seed in context so the public pages render their first paint from
/// it (no reactive fetch) via the same `render` fn the projector used — coincident,
/// flash-free. On the static SPA shell (no blob, no `#app`) the seed is `None` and
/// this is an ordinary `mount_to_body`.
fn mount() {
    let presentation = projector_seed();
    perf::mark(perf::BOOT_SEED_PARSED);
    // App re-renders the identical content from `seed`, so removing the
    // server-painted copy avoids a duplicate paint without a visible flash (the
    // removal and remount happen in one synchronous task).
    dom::remove_element_by_id("app");
    // Drop the projector-painted discovery <link>s so the reactive FeedDiscovery/
    // RsdDiscovery mounted below produce the ONLY set (no invisible duplicate). Crawlers/
    // no-JS never run this, so their head is unchanged (#198).
    dom::remove_elements_by_selector(&format!("link[{}]", web::app::DISCOVERY_MARKER_ATTR));
    perf::mark(perf::BOOT_RENDER_START);
    leptos::mount::mount_to_body(move || {
        provide_context(presentation.as_ref().map(|value| value.page.clone()));
        provide_context(RwSignal::new(
            presentation
                .as_ref()
                .map_or(common::theme::Theme::Studio, |value| value.theme),
        ));
        view! { <App /> }
    });
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    // First statement: `BOOT_ENTRY` is the wasm's own "I am running" timestamp, and
    // the harness derives fetch/compile/instantiate from the gap before it.
    perf::mark(perf::BOOT_ENTRY);
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    mount();
    perf::mark(perf::BOOT_MOUNT_DONE);
    mark_ready();
}
