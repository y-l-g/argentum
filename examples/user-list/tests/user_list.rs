//! S2 — end-to-end page render: a `#[page]` lists Toasty User rows seeded into
//! `sqlite::memory:` through the full model → query → view → response path.

use http_body_util::BodyExt;
use toasty::Db;
use topcoat::router::{Body, Router, RouterBuilderDiscoverExt};

use user_list::models::{self, User};

async fn seeded_router() -> Router {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");
    db.push_schema().await.expect("push schema");
    models::seed(&mut db).await.expect("seed");

    Router::builder().discover().app_context(db).build()
}

#[tokio::test]
async fn page_renders_seeded_users() {
    let router = seeded_router().await;

    let response = router
        .handle(
            http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();

    assert!(status.is_success(), "status was {status}");
    let html = String::from_utf8_lossy(&body);
    for (name, email) in [
        ("Ada Lovelace", "ada@example.com"),
        ("Grace Hopper", "grace@example.com"),
        ("Linus Torvalds", "linus@example.com"),
    ] {
        assert!(html.contains(name), "missing {name} in {html}");
        assert!(html.contains(email), "missing {email} in {html}");
    }
}
