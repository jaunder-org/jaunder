#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function install_fetch_timing() {
        if (typeof window === 'undefined' || typeof window.fetch !== 'function') {
            return;
        }
        if (window.__jaunder_fetch_timing_installed) {
            return;
        }
        window.__jaunder_fetch_timing_installed = true;

        const originalFetch = window.fetch.bind(window);
        window.fetch = async function(input, init) {
            const startedAt = (typeof performance !== 'undefined' && performance.now)
                ? performance.now()
                : Date.now();
            let response;
            try {
                response = await originalFetch(input, init);
            } catch (error) {
                const endedAt = (typeof performance !== 'undefined' && performance.now)
                    ? performance.now()
                    : Date.now();
                const durationMs = endedAt - startedAt;
                console.info(
                    '[jaunder-fetch]',
                    JSON.stringify({
                        ok: false,
                        method: (init && init.method) || 'GET',
                        url: String(input && input.url ? input.url : input),
                        duration_ms: durationMs,
                        error: String(error),
                    })
                );
                throw error;
            }

            const endedAt = (typeof performance !== 'undefined' && performance.now)
                ? performance.now()
                : Date.now();
            const durationMs = endedAt - startedAt;
            const requestId = response.headers.get('x-request-id');
            console.info(
                '[jaunder-fetch]',
                JSON.stringify({
                    ok: response.ok,
                    status: response.status,
                    method: (init && init.method) || 'GET',
                    url: response.url || String(input && input.url ? input.url : input),
                    duration_ms: durationMs,
                    request_id: requestId,
                })
            );
            return response;
        };
    }

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
    fn install_fetch_timing();
    fn mark_hydration_start();
    fn mark_hydrated();
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use web::*;
    // initializes logging using the `log` crate
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();

    install_fetch_timing();
    mark_hydration_start();
    leptos::mount::hydrate_body(App);

    // Signal to e2e tests that hydration (including all initial reactive
    // effects such as `prop:value` bindings) has completed.
    mark_hydrated();
}
