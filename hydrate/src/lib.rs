#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_hydrated() {
        document.body.setAttribute('data-hydrated', 'true');
    }
")]
extern "C" {
    fn mark_hydrated();
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use web::*;
    // initializes logging using the `log` crate
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();

    leptos::mount::hydrate_body(App);

    // Signal to e2e tests that hydration (including all initial reactive
    // effects such as `prop:value` bindings) has completed.
    mark_hydrated();
}
