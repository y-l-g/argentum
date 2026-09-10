//! Shared fixtures and request scaffolding for the showcase integration
//! tests (GH #111). Every test crate includes this module via `mod common;`
//! and uses a subset of it, so `dead_code` is expected here and allowed once
//! instead of leaking per-crate warnings.

#![allow(dead_code)]

use argentum_core::Resource;
use http::header::{CONTENT_TYPE, COOKIE};
use http_body_util::BodyExt;
use showcase::models::{seed, seed_phase2};
use toasty::Db;
use topcoat::context::Cx;
use topcoat::router::{Body, Router};

/// `Db` with the phase-1 users seed applied.
pub async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(showcase::models::User))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    db
}

/// `Db` with both seed phases (users, authors, posts, comments).
pub async fn full_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    seed_phase2(&mut db).await.expect("seed_phase2");
    db
}

/// `Db` with one author and post per tenant, for tenancy tests.
pub async fn tenanted_db() -> (Db, uuid::Uuid, uuid::Uuid) {
    let t1 = uuid::Uuid::from_u128(1);
    let t2 = uuid::Uuid::from_u128(2);
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    let a1 = toasty::create!(showcase::models::Author {
        tenant_id: t1,
        name: "Alice T1",
        email: "alice.t1@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create author t1");
    let a2 = toasty::create!(showcase::models::Author {
        tenant_id: t2,
        name: "Bob T2",
        email: "bob.t2@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create author t2");
    toasty::create!(showcase::models::Post {
        tenant_id: t1,
        title: "T1 Post",
        body: "body",
        status: "published".to_string(),
        featured: true,
        created_at: "2024-01-15T09:30:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t1.jpg".to_string(),
        tags: "t1".to_string(),
        author_id: a1.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t1");
    toasty::create!(showcase::models::Post {
        tenant_id: t2,
        title: "T2 Post",
        body: "body",
        status: "draft".to_string(),
        featured: false,
        created_at: "2024-06-01T12:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t2.jpg".to_string(),
        tags: "t2".to_string(),
        author_id: a2.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t2");
    (db, t1, t2)
}

/// GET `uri`.
pub async fn get(router: &Router, uri: &str) -> http::Response<Body> {
    router
        .handle(
            http::Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
}

/// GET `uri` carrying the `x-tenant-id` header.
pub async fn get_tenant(router: &Router, uri: &str, tenant: uuid::Uuid) -> http::Response<Body> {
    router
        .handle(
            http::Request::builder()
                .uri(uri)
                .header("x-tenant-id", tenant.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
}

/// POST an urlencoded form authenticated with the CSRF cookie + token.
pub async fn post_form(
    router: &Router,
    uri: &str,
    csrf: &str,
    body: String,
) -> http::Response<Body> {
    router
        .handle(
            http::Request::builder()
                .uri(uri)
                .method(http::Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
}

/// POST an urlencoded form carrying the `x-tenant-id` header.
pub async fn post_form_tenant(
    router: &Router,
    uri: &str,
    tenant: uuid::Uuid,
    csrf: &str,
    body: String,
) -> http::Response<Body> {
    router
        .handle(
            http::Request::builder()
                .uri(uri)
                .method(http::Method::POST)
                .header("x-tenant-id", tenant.to_string())
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
}

/// POST a multipart body carrying the `x-tenant-id` header (file uploads).
pub async fn post_multipart_tenant(
    router: &Router,
    uri: &str,
    tenant: uuid::Uuid,
    csrf: &str,
    boundary: &str,
    body: String,
) -> http::Response<Body> {
    router
        .handle(
            http::Request::builder()
                .uri(uri)
                .method(http::Method::POST)
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .header("x-tenant-id", tenant.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await
}

/// Collect a response body as a lossy UTF-8 string.
pub async fn body_string(response: http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Assert every hydrated key is a declared form field (GH #89): a renamed
/// lens without an updated string literal would render blank and break the
/// unique unchanged-skip.
pub fn assert_hydrate_keys_are_form_fields<R: Resource>(cx: &Cx, record: &R::Model) {
    let fields = R::form(cx).field_names();
    for key in R::hydrate_form_values(record).keys() {
        assert!(
            fields.contains(key),
            "hydrate key {key} is not a {} form field (GH #89)",
            R::slug()
        );
    }
}
