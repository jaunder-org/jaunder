use crate::{
    auth::current_user,
    posts::{CreatePost, CreatePostResult},
};
use leptos::prelude::*;

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

                        view! {
                            <p class="success">{message}</p>
                            <p>"Slug: " {created.slug}</p>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}
