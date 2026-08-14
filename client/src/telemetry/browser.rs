//! Browser console and credentialed keepalive-fetch adapter.

use common::client_telemetry::{
    ClientErrorContext, ClientErrorKind, ClientSourceKind, ClientTelemetryEvent,
};
use js_sys::Reflect;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Request, RequestCredentials, RequestInit};

use super::{Completion, ConsoleSink, Reporter, Transport};

const ENDPOINT: &str = "/api/client-telemetry";
const LOCAL_WARNING: &str = "jaunder swallowed browser error";
const DELIVERY_WARNING: &str = "jaunder client diagnostic delivery failed";

std::thread_local! {
    static REPORTER: Reporter<FetchTransport> = Reporter::new(FetchTransport, BrowserConsole);
}

pub fn report_swallowed(
    kind: ClientErrorKind,
    context: ClientErrorContext,
    source_kind: ClientSourceKind,
) {
    REPORTER.with(|reporter| reporter.report_swallowed(kind, context, source_kind));
}

struct BrowserConsole;

impl ConsoleSink for BrowserConsole {
    fn warn(&self, _event: &ClientTelemetryEvent) {
        web_sys::console::warn_1(&JsValue::from_str(LOCAL_WARNING));
    }
}

struct FetchTransport;

impl Transport for FetchTransport {
    fn send(&self, event: ClientTelemetryEvent, on_complete: Completion) {
        let request = match request_for(event) {
            Ok(request) => request,
            Err(error) => {
                warn_delivery_failure(&error);
                on_complete();
                return;
            }
        };
        let Some(window) = web_sys::window() else {
            warn_delivery_failure(&JsValue::from_str("browser window is unavailable"));
            on_complete();
            return;
        };
        let pending = window.fetch_with_request(&request);

        spawn_local(async move {
            if let Err(error) = JsFuture::from(pending).await {
                warn_delivery_failure(&error);
            }
            on_complete();
        });
    }
}

fn request_for(event: ClientTelemetryEvent) -> Result<Request, JsValue> {
    let body = serialize_event(event)?;
    let init = configured_request_init(&body)?;
    request_with_json_header(&init)
}

fn serialize_event(event: ClientTelemetryEvent) -> Result<String, JsValue> {
    serde_json::to_string(&event).map_err(|error| JsValue::from_str(&error.to_string()))
}

fn configured_request_init(body: &str) -> Result<RequestInit, JsValue> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(body));
    init.set_credentials(RequestCredentials::Include);

    let keepalive = JsValue::from_str("keepalive");
    let enabled = JsValue::from_bool(true);
    if !Reflect::set(init.as_ref(), &keepalive, &enabled)? {
        return Err(JsValue::from_str("cannot enable fetch keepalive"));
    }

    Ok(init)
}

fn request_with_json_header(init: &RequestInit) -> Result<Request, JsValue> {
    let request = Request::new_with_str_and_init(ENDPOINT, init)?;
    request.headers().set("Content-Type", "application/json")?;
    Ok(request)
}

fn warn_delivery_failure(error: &JsValue) {
    // The transport's local fallback cannot use the reporter without recursively
    // consuming the same one-flight path it is trying to release.
    web_sys::console::warn_2(&JsValue::from_str(DELIVERY_WARNING), error);
}
