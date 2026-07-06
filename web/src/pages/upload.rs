use leptos::prelude::*;

/// A button that lets the user pick a file and immediately uploads it to
/// `/media/upload` via JavaScript fetch (no page navigation).
///
/// `on_uploaded` is called with the media URL string on success.
/// `on_error` is called with a human-readable message on failure.
#[component]
pub fn MediaUploadButton(
    /// Called with the `/media/upload/...` URL when the upload succeeds.
    #[prop(into)]
    on_uploaded: Callback<String>,
    /// Called with an error message when the upload fails.
    #[prop(into, optional)]
    on_error: Option<Callback<String>>,
) -> impl IntoView {
    // `on_uploaded`/`on_error` are consumed only in the wasm upload path below;
    // acknowledge them on the SSR build so they don't read as unused.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (&on_uploaded, &on_error);

    let uploading = RwSignal::new(false);
    let file_input = NodeRef::<leptos::html::Input>::new();

    let open_picker = move |_| {
        if let Some(input) = file_input.get() {
            input.click();
        }
    };

    let on_file_change = move |ev: leptos::ev::Event| {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ev;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ev;
            use leptos::task::spawn_local;
            use leptos::wasm_bindgen::JsCast;

            let Some(input) = file_input.get() else {
                return;
            };
            let input_el: web_sys::HtmlInputElement = input.unchecked_into();
            let Some(files) = input_el.files() else {
                return;
            };
            let Some(file): Option<web_sys::File> = files.get(0) else {
                return;
            };

            let form_data = match web_sys::FormData::new() {
                Ok(fd) => fd,
                Err(_) => return,
            };
            if form_data.append_with_blob("file", &file).is_err() {
                return;
            }

            uploading.set(true);

            spawn_local(async move {
                let result = upload_file(form_data).await;
                uploading.set(false);
                match result {
                    Ok(url) => on_uploaded.run(url),
                    Err(msg) => {
                        if let Some(cb) = on_error {
                            cb.run(msg);
                        }
                    }
                }
            });
        }
    };

    view! {
        <input type="file" node_ref=file_input style="display:none" on:change=on_file_change />
        <button type="button" class="j-btn" disabled=move || uploading.get() on:click=open_picker>
            {move || if uploading.get() { "Uploading\u{2026}" } else { "Attach media" }}
        </button>
    }
}

/// Self-contained media upload widget: button, uploaded-URL display, and error.
/// Drop this into any `ActionForm` aside that needs media upload.
#[component]
pub fn MediaPanel() -> impl IntoView {
    let last_media_url = RwSignal::new(Option::<String>::None);
    let upload_error = RwSignal::new(Option::<String>::None);

    view! {
        <MediaUploadButton
            on_uploaded=Callback::new(move |url: String| {
                last_media_url.set(Some(url));
                upload_error.set(None);
            })
            on_error=Callback::new(move |msg: String| {
                upload_error.set(Some(msg));
            })
        />
        {move || {
            last_media_url
                .get()
                .map(|url| {
                    view! {
                        <div style="margin-top:8px">
                            <div style="font-size:12px;color:#888;margin-bottom:4px">
                                "Uploaded URL:"
                            </div>
                            <input
                                type="text"
                                readonly
                                value=url.clone()
                                class="j-field-val"
                                style="font-size:12px;cursor:text"
                                on:click=move |ev| {
                                    use leptos::wasm_bindgen::JsCast;
                                    let _ = ev
                                        .target()
                                        .and_then(|t| {
                                            t.dyn_into::<web_sys::HtmlInputElement>().ok()
                                        })
                                        .map(|i| i.select());
                                }
                            />
                        </div>
                    }
                })
        }}
        {move || {
            upload_error
                .get()
                .map(|msg| {
                    view! {
                        <p class="error" style="margin-top:6px;font-size:12px">
                            {msg}
                        </p>
                    }
                })
        }}
    }
}

#[cfg(target_arch = "wasm32")]
async fn upload_file(form_data: web_sys::FormData) -> Result<String, String> {
    use leptos::wasm_bindgen::JsCast;
    use leptos::wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;

    let mut opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    let body_val: JsValue = form_data.into();
    opts.set_body(&body_val);

    let request = web_sys::Request::new_with_str_and_init("/media/upload", &opts).map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| "failed to build request".to_string())
    })?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "network error".to_string()))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "unexpected response type".to_string())?;

    if !resp.ok() {
        return Err(format!("upload failed (HTTP {})", resp.status()));
    }

    let text_promise = resp.text().map_err(|_| "failed to read response body")?;
    let text_value: JsValue = JsFuture::from(text_promise)
        .await
        .map_err(|_| "failed to await response text")?;

    let body: String = text_value
        .as_string()
        .ok_or_else(|| "response body is not a string".to_string())?;

    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| "invalid JSON in response".to_string())?;

    parsed["url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "response JSON missing 'url' field".to_string())
}
