// web::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors web/src/lib.rs).
#![recursion_limit = "512"]

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

#[wasm_bindgen::prelude::wasm_bindgen(start)]
// cov:ignore-start
pub fn main() {
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    // cov:ignore-stop
    // Boots the App, adopting the public projector's data blob when present
    // (#178/#179). See `web::mount_csr`.
    // cov:ignore-start
    web::mount_csr();
    mark_ready();
}
// cov:ignore-stop
