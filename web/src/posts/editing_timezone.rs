//! Submission-time device capture and its persistent, navigation-safe warning.

use common::time::PublicationTimezone;
use js_sys::{Array, Function, Reflect};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

/// Shell-owned so a successful published edit cannot navigate away its warning.
#[derive(Clone, Copy)]
pub(crate) struct EditingTimezoneNotice(RwSignal<bool>);

impl EditingTimezoneNotice {
    pub(crate) fn new() -> Self {
        Self(RwSignal::new(false))
    }

    /// Capture anew on every authored submission, never when loading a Post.
    pub(crate) fn capture(self) -> PublicationTimezone {
        let zone = device_timezone().and_then(|name| name.parse().ok());
        self.0.set(zone.is_none());
        zone.unwrap_or_else(PublicationTimezone::utc)
    }
}

// Reflect's fallible calls also cover missing Intl and exceptions from the device
// boundary, rather than letting a JavaScript exception abort a save.
fn intl_formatter() -> Option<JsValue> {
    let intl = Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let constructor = Reflect::get(&intl, &JsValue::from_str("DateTimeFormat"))
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    Reflect::construct(&constructor, &Array::new()).ok()
}

fn resolved_intl_options(formatter: &JsValue) -> Option<JsValue> {
    let method = Reflect::get(formatter, &JsValue::from_str("resolvedOptions"))
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    method.call0(formatter).ok()
}

fn device_timezone() -> Option<String> {
    let formatter = intl_formatter()?;
    let resolved = resolved_intl_options(&formatter)?;
    Reflect::get(&resolved, &JsValue::from_str("timeZone"))
        .ok()?
        .as_string()
}

#[component]
pub(crate) fn EditingTimezoneBanner() -> impl IntoView {
    let notice = expect_context::<EditingTimezoneNotice>();
    view! {
        <Show when=move || notice.0.get()>
            <p class="error" role="alert" data-test="editing-timezone-warning">
                "Your device timezone could not be determined or is unsupported. UTC was used for this save."
            </p>
        </Show>
    }
}
