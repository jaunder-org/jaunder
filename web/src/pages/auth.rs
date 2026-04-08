use crate::auth::{get_registration_policy, Login, Logout, Register};
use leptos::prelude::*;

/// Registration page.
#[component]
pub fn RegisterPage() -> impl IntoView {
    let register_action = ServerAction::<Register>::new();
    let policy = Resource::new(|| (), |_| get_registration_policy());
    let username = RwSignal::new(String::new());

    view! {
        <h1>"Register"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                let p = policy.await;
                let is_invite_only = p.as_deref() == Ok("invite_only");
                view! {
                    <ActionForm action=register_action>
                        <label>
                            "Username"
                            <input
                                type="text"
                                name="username"
                                prop:value=username
                                on:input=move |ev| {
                                    username.set(event_target_value(&ev).to_lowercase());
                                }
                            />
                        </label>
                        <label>"Password" <input type="password" name="password" /></label>
                        {is_invite_only
                            .then(|| {
                                view! {
                                    <label>
                                        "Invite code" <input type="text" name="invite_code" />
                                    </label>
                                }
                            })}
                        <button type="submit">"Register"</button>
                    </ActionForm>
                }
            })}
        </Suspense>
        {move || {
            register_action
                .value()
                .get()
                .and_then(|r: Result<String, ServerFnError>| r.err())
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let username = RwSignal::new(String::new());

    view! {
        <h1>"Login"</h1>
        <ActionForm action=login_action>
            <label>
                "Username"
                <input
                    type="text"
                    name="username"
                    prop:value=username
                    on:input=move |ev| {
                        username.set(event_target_value(&ev).to_lowercase());
                    }
                />
            </label>
            <label>"Password" <input type="password" name="password" /></label>
            <button type="submit">"Login"</button>
        </ActionForm>
        {move || {
            login_action
                .value()
                .get()
                .and_then(|r: Result<String, ServerFnError>| r.err())
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}

/// Logout page — fires the logout server action on mount.
#[component]
pub fn LogoutPage() -> impl IntoView {
    let logout_action = ServerAction::<Logout>::new();

    Effect::new(move |_| {
        logout_action.dispatch(Logout {});
    });

    view! {
        <h1>"Logging out\u{2026}"</h1>
        {move || {
            logout_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| {
                    match r {
                        Ok(_) => view! { <p>"You have been logged out."</p> }.into_any(),
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                    }
                })
        }}
    }
}
