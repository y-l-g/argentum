use axum::{Router, routing::get};
use maud::{DOCTYPE, html};

async fn home() -> maud::Markup {
    html! {
        (DOCTYPE)
        html {
            head { title { "Axum + Maud bench stub" } }
            body { h1 { "Benchmark stub" } }
        }
    }
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(home));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8090")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}
