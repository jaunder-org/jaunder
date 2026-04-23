// The ParentRoute wrapping all routes in web::App generates a wide tuple of
// route types; the compiler needs a higher recursion limit to monomorphize it,
// particularly under llvm-cov instrumentation. Root cause under investigation.
#![recursion_limit = "512"]

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_phase_start(name) {
        if (typeof performance === 'undefined') {
            return;
        }
        performance.mark(`jaunder_phase_${name}_start`);
    }

    export function mark_phase_end(name) {
        if (typeof performance === 'undefined') {
            return;
        }
        const endMark = `jaunder_phase_${name}_end`;
        const startMark = `jaunder_phase_${name}_start`;
        const measureName = `jaunder_phase_${name}`;
        performance.mark(endMark);
        try {
            performance.measure(measureName, startMark, endMark);
        } catch (_err) {
            // Ignore if marks are missing or unsupported.
        }
    }

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
        if (document && document.body) {
            document.body.setAttribute('data-hydrated', 'true');
        }

        if (typeof performance === 'undefined') {
            return;
        }

        try {
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
                wasm_init_ms: (() => {
                    const entry = performance.getEntriesByName('jaunder_phase_wasm_init').slice(-1)[0];
                    return entry ? entry.duration : null;
                })(),
                leptos_hydrate_ms: (() => {
                    const entry = performance.getEntriesByName('jaunder_phase_leptos_hydrate').slice(-1)[0];
                    return entry ? entry.duration : null;
                })(),
                post_hydrate_effects_ms: (() => {
                    const measure = performance.getEntriesByName('jaunder_phase_post_hydrate_effects').slice(-1)[0];
                    if (measure) {
                        return measure.duration;
                    }
                    // mark_hydrated runs before the post_hydrate_effects phase ends,
                    // so the measure may not exist yet. Fall back to elapsed time from
                    // the phase start mark to avoid reporting null.
                    const start = performance
                        .getEntriesByName('jaunder_phase_post_hydrate_effects_start')
                        .slice(-1)[0];
                    return start ? performance.now() - start.startTime : null;
                })(),
            };

            window.__jaunder_perf = payload;
            console.info('[jaunder-perf]', JSON.stringify(payload));
        } catch (_err) {
            // Never let instrumentation break hydration signaling.
        }
    }
")]
extern "C" {
    fn install_fetch_timing();
    fn mark_phase_start(name: &str);
    fn mark_phase_end(name: &str);
    fn mark_hydration_start();
    fn mark_hydrated();
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use web::*;
    mark_phase_start("wasm_init");
    // initializes logging using the `log` crate
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    mark_phase_end("wasm_init");

    install_fetch_timing();
    mark_hydration_start();
    mark_phase_start("leptos_hydrate");
    leptos::mount::hydrate_body(App);
    mark_phase_end("leptos_hydrate");

    // Signal to e2e tests that hydration (including all initial reactive
    // effects such as `prop:value` bindings) has completed.
    mark_phase_start("post_hydrate_effects");
    mark_hydrated();
    mark_phase_end("post_hydrate_effects");
}
