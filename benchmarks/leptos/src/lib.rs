use leptos::prelude::*;

/// The shared document shell, wrapping the app's routes for SSR.
#[component]
pub fn shell(leptos_options: LeptosOptions, children: Children) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1.0" />
                <title>"Leptos bench stub"</title>
                <link rel="stylesheet" id="leptos" href="/pkg/storefront-leptos.css" />
            </head>
            <body>
                {
                    children()
                }
            </body>
        </html>
    }
}

/// The root view. Stub: a single heading, comparable to the other stubs.
#[component]
pub fn App() -> impl IntoView {
    view! { <h1>"Benchmark stub"</h1> }
}
