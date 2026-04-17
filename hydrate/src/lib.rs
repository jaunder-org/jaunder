#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_hydration_start() {
        if (typeof performance !== 'undefined') {
            performance.mark('jaunder_hydration_start');
        }
    }

    export function mark_hydrated() {
        document.body.setAttribute('data-hydrated', 'true');

        if (typeof performance === 'undefined') {
            return;
        }

        performance.mark('jaunder_hydration_end');
        try {
            performance.measure(
                'jaunder_hydration',
                'jaunder_hydration_start',
                'jaunder_hydration_end'
            );
        } catch (_err) {
            // Ignore if marks are missing or unsupported.
        }

        const hydrationMeasure = performance
            .getEntriesByName('jaunder_hydration')
            .slice(-1)[0];
        const navigationEntry = performance.getEntriesByType('navigation')[0];
        const wasmEntry = performance
            .getEntriesByType('resource')
            .find((entry) => entry.name.includes('jaunder_bg.wasm'));

        const payload = {
            hydration_ms: hydrationMeasure ? hydrationMeasure.duration : null,
            navigation_ms: navigationEntry ? navigationEntry.duration : null,
            wasm_transfer_bytes: wasmEntry ? wasmEntry.transferSize : null,
            wasm_resource_ms: wasmEntry ? wasmEntry.duration : null,
        };

        window.__jaunder_perf = payload;
        console.info('[jaunder-perf]', JSON.stringify(payload));
    }
")]
extern "C" {
    fn mark_hydration_start();
    fn mark_hydrated();
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use web::*;
    // initializes logging using the `log` crate
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();

    mark_hydration_start();
    leptos::mount::hydrate_body(App);

    // Signal to e2e tests that hydration (including all initial reactive
    // effects such as `prop:value` bindings) has completed.
    mark_hydrated();
}
