use leptos::prelude::*;

#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <section>
            <h1>"Jaunder"</h1>
            <p>"A self-hosted social reader."</p>
            <nav>
                <a href="/login">"Login"</a>
                " "
                <a href="/register">"Register"</a>
            </nav>
        </section>
    }
}
