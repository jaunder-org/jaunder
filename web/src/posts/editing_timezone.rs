//! Submission-time device capture and its persistent, navigation-safe warning.

use common::time::PublicationTimezone;
use js_sys::{Array, Function, Reflect};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

/// Shell-owned so a successful published edit cannot navigate away its warning.
#[derive(Clone, Copy)]
pub(crate) struct EditingTimezoneNotice(RwSignal<Option<String>>);

impl EditingTimezoneNotice {
    pub(crate) fn new() -> Self {
        Self(RwSignal::new(None))
    }

    /// Capture anew on every authored submission, never when loading a Post.
    pub(crate) fn capture(self) -> PublicationTimezone {
        if let Some(zone) = device_timezone().and_then(|name| name.parse().ok()) {
            self.0.set(None);
            return zone;
        }
        if let Some(zone) =
            device_offset_seconds_east().and_then(PublicationTimezone::from_device_offset_seconds)
        {
            self.0.set(Some(format!(
                "Your device timezone name is unavailable or unsupported. Inferred fixed offset {zone} was used for this save; regional timezone and DST history are unknown."
            )));
            return zone;
        }
        self.0.set(Some("Your device timezone and an exact supported whole-hour offset could not be determined. UTC was used for this save.".to_owned()));
        PublicationTimezone::utc()
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

// Both device APIs need fallible lookup and invocation with their own receiver.
fn call_device_method(receiver: &JsValue, name: &str) -> Option<JsValue> {
    let method = Reflect::get(receiver, &JsValue::from_str(name))
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    method.call0(receiver).ok()
}

fn device_timezone() -> Option<String> {
    let formatter = intl_formatter()?;
    let resolved = call_device_method(&formatter, "resolvedOptions")?;
    Reflect::get(&resolved, &JsValue::from_str("timeZone"))
        .ok()?
        .as_string()
}

// Date is independent of Intl: even a throwing/missing formatter can still
// supply the current device offset. JS reports minutes WEST of UTC, not east.
fn device_offset_seconds_east() -> Option<i32> {
    let date = js_sys::Date::new_0();
    // Exact integer parsing rejects NaN, infinities and fractional minutes
    // rather than using a float-to-integer cast that could truncate them.
    let minutes_west: i32 = call_device_method(&date, "getTimezoneOffset")?
        .as_f64()?
        .to_string()
        .parse()
        .ok()?;
    // The shared PublicationTimezone capture constructor owns whole-hour/range
    // validation; this boundary only converts units/sign without overflow.
    minutes_west.checked_mul(-60)
}

#[component]
pub(crate) fn EditingTimezoneBanner() -> impl IntoView {
    let notice = expect_context::<EditingTimezoneNotice>();
    view! {
        <Show when=move || notice.0.get().is_some()>
            <p class="error" role="alert" data-test="editing-timezone-warning">
                {move || notice.0.get()}
            </p>
        </Show>
    }
}
