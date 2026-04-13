use crate::{
    auth::current_user,
    posts::{get_post, get_post_preview, CreatePost, CreatePostResult, PostResponse},
};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn CreatePostPage() -> impl IntoView {
    let create_post_action = ServerAction::<CreatePost>::new();
    let current_user = Resource::new(|| (), |_| current_user());

    view! {
        <h1>"New Post"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match current_user.await {
                    Ok(Some(_username)) => {
                        view! {
                            <ActionForm action=create_post_action>
                                <label>
                                    "Title" <input type="text" name="title" required=true />
                                </label>
                                <label>"Body" <textarea name="body" rows="12"></textarea></label>
                                <label>
                                    "Format" <select name="format">
                                        <option value="markdown" selected=true>
                                            "Markdown"
                                        </option>
                                        <option value="org">"Org"</option>
                                    </select>
                                </label>
                                <label>
                                    "Slug override" <input type="text" name="slug_override" />
                                </label>
                                <button type="submit" name="publish" value="true">
                                    "Publish"
                                </button>
                                <button type="submit" name="publish" value="false">
                                    "Save Draft"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    }
                    Ok(None) => {
                        view! {
                            <p>"You must be logged in to create a post."</p>
                            <p>
                                <a href="/login">"Login"</a>
                            </p>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            create_post_action
                .value()
                .get()
                .map(|result: Result<CreatePostResult, ServerFnError>| match result {
                    Ok(created) => {
                        let message = if created.published_at.is_some() {
                            "Post published."
                        } else {
                            "Draft saved."
                        };
                        let slug_value = created.slug.clone();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="success">
                                <p>{message}</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="preview-link" href=created.preview_url.clone()>
                                    "Preview draft"
                                </a>
                                {created
                                    .permalink
                                    .as_ref()
                                    .map(|href| {
                                        view! {
                                            <a data-test="permalink-link" href=href.clone()>
                                                "View permalink"
                                            </a>
                                        }
                                    })}
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

#[component]
pub fn PostPage() -> impl IntoView {
    let params = use_params_map();

    let post_data = move || {
        let params = params.get();
        let raw_username = params.get("username").unwrap_or_default();
        let username = raw_username.strip_prefix('~').map(str::to_string);
        let year = params
            .get("year")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or_default();
        let month = params
            .get("month")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let day = params
            .get("day")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let slug = params.get("slug").unwrap_or_default();

        (username, year, month, day, slug)
    };

    let post = Resource::new(
        post_data,
        |(username, year, month, day, slug): (Option<String>, i32, u32, u32, String)| async move {
            let username = match username {
                Some(value) if !value.is_empty() => value,
                _ => return Err(ServerFnError::new("Invalid permalink")),
            };

            get_post(username, year, month, day, slug).await
        },
    );

    view! {
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched_post) => render_post_article(fetched_post, None),
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
pub fn DraftPreviewPage() -> impl IntoView {
    let params = use_params_map();

    let preview = Resource::new(
        move || params.get(),
        |params| async move {
            let post_id = params
                .get("post_id")
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| ServerFnError::new("Invalid preview"))?;
            get_post_preview(post_id).await
        },
    );

    view! {
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match preview.await {
                    Ok(post) => {
                        render_post_article(post, Some("Draft preview – visible only to you"))
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

fn render_post_article(post: PostResponse, banner: Option<&'static str>) -> AnyView {
    let PostResponse {
        title,
        username,
        rendered_html,
        created_at,
        ..
    } = post;
    let profile_href = format!("/~{}/", username);
    let username_display = username.clone();

    view! {
        <article>
            <h1>{title}</h1>
            <p class="metadata">
                "By " <a href=profile_href>{username_display}</a> " on " {created_at}
            </p>
            {banner.map(|text| view! { <p class="draft-banner">{text}</p> })}
            <div class="content" inner_html=rendered_html></div>
        </article>
    }
    .into_any()
}
