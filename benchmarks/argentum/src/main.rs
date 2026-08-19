use topcoat::{
    Result,
    router::{Router, RouterBuilderDiscoverExt, page},
    view::view,
};

use argentum_core::view::heading;

#[tokio::main]
async fn main() {
    topcoat::start(Router::builder().discover().build())
        .await
        .unwrap();
}

#[page("/")]
async fn home() -> Result {
    view! {
        <!DOCTYPE html>
        <html>
            <head><title>"Argentum bench stub"</title></head>
            <body>heading(title: "Benchmark stub")</body>
        </html>
    }
}
