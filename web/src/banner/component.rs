use crate::error::WebResult;
use leptos::prelude::*;

/// A sticky operational warning bar (`role="alert"`, `.j-warn-banner`) shown only
/// when `visible` resolves to `Ok(true)`. `message` is the alert copy; `links` are
/// `(href, label)` action links rendered in the banner's link row. Shared by the
/// backup and site verticals — the visibility server fn, copy, and links are the
/// caller's; the structure and styling live here so the two banners stay identical.
#[component]
pub fn WarnBanner(
    visible: Resource<WebResult<bool>>,
    message: &'static str,
    links: Vec<(&'static str, &'static str)>,
) -> impl IntoView {
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let links = links.clone();
                Suspend::new(async move {
                    match visible.await {
                        Ok(true) => {
                            let items = links
                                .iter()
                                .map(|(href, label)| view! { <a href=*href>{*label}</a> })
                                .collect_view();
                            view! {
                                <div class="j-warn-banner" role="alert">
                                    <span>{message}</span>
                                    <div>{items}</div>
                                </div>
                            }
                                .into_any()
                        }
                        _ => ().into_any(),
                    }
                })
            }}
        </Suspense>
    }
}
