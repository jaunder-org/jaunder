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

// Date is independent of Intl: even a throwing/missing formatter can still
// supply the current device offset. JS reports minutes WEST of UTC, not east.
fn device_offset_seconds_east() -> Option<i32> {
    let date = js_sys::Date::new_0();
    let method = Reflect::get(&date, &JsValue::from_str("getTimezoneOffset"))
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    // Exact integer parsing rejects NaN, infinities and fractional minutes
    // rather than using a float-to-integer cast that could truncate them.
    let minutes_west: i32 = method
        .call0(&date)
        .ok()?
        .as_f64()?
        .to_string()
        .parse()
        .ok()?;
    if !(-720..=720).contains(&minutes_west) || minutes_west % 60 != 0 {
        return None;
    }
    Some(-minutes_west * 60)
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
