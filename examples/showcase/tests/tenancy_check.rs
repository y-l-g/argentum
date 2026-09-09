use http::Request;
use http_body_util::BodyExt;
use showcase::{
    app::router_for_tests as router,
    models::{Author, Post},
};
use toasty::Db;
use topcoat::router::Body;

async fn tenanted_db() -> (Db, uuid::Uuid, uuid::Uuid) {
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
        .unwrap();
    db.push_schema().await.unwrap();
    // Seed tenant-specific data
    let a1 = toasty::create!(Author {
        tenant_id: t1,
        name: "Alice T1",
        email: "alice.t1@example.com",
    })
    .exec(&mut db)
    .await
    .unwrap();
    let a2 = toasty::create!(Author {
        tenant_id: t2,
        name: "Bob T2",
        email: "bob.t2@example.com",
    })
    .exec(&mut db)
    .await
    .unwrap();
    toasty::create!(Post {
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
    .unwrap();
    toasty::create!(Post {
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
    .unwrap();
    (db, t1, t2)
}

#[tokio::test]
async fn posts_list_is_scoped_by_tenant_via_resource_query() {
    let (db, t1, t2) = tenanted_db().await;
    let router = router(db.clone());

    let resp_t1 = router
        .handle(
            Request::builder()
                .uri("/admin/posts")
                .header("x-tenant-id", t1.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp_t1.status().is_success());
    let body = resp_t1.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("T1 Post"), "t1 should see T1 Post {}", html);
    assert!(
        !html.contains("T2 Post"),
        "t1 should not see T2 Post {}",
        html
    );

    let resp_t2 = router
        .handle(
            Request::builder()
                .uri("/admin/posts")
                .header("x-tenant-id", t2.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp_t2.status().is_success());
    let body = resp_t2.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("T2 Post"), "t2 should see T2 Post {}", html);
    assert!(
        !html.contains("T1 Post"),
        "t2 should not see T1 Post {}",
        html
    );
}

#[tokio::test]
async fn edit_with_wrong_tenant_yields_404_via_resource_query() {
    let (db, t1, t2) = tenanted_db().await;
    let router = router(db.clone());
    // Find T1 post id
    let mut db2 = db.clone();
    let t1_post = Post::filter(Post::fields().tenant_id().eq(t1))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .unwrap();
    let edit_url = format!("/admin/posts/{}/edit", t1_post.id);
    // Try to edit with t2 tenant -> should be 404 (not found via query)
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url)
                .header("x-tenant-id", t2.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "wrong tenant should be 404, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn per_tenant_policy_deny_yields_403() {
    let (db, _, _) = tenanted_db().await;
    let router = router(db);
    let blocked = uuid::Uuid::from_u128(9999);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts")
                .header("x-tenant-id", blocked.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "blocked tenant should be 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn tenancy_via_cx_with_tenant_scopes_query_directly() {
    use argentum_core::{Resource, Tenant};
    use showcase::app::PostResource;
    use topcoat::context::CxTestBuilder;
    let (db, t1, _) = tenanted_db().await;
    let cx_t1 = CxTestBuilder::new()
        .app_context(db.clone())
        .request_context(Tenant(t1))
        .build();
    let mut db_cx = argentum_core::db::db(&cx_t1);
    let rows = PostResource::query(&cx_t1).exec(&mut db_cx).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "T1 Post");

    // Different tenant via Cx::with
    let cx_t2 = cx_t1.with(Tenant(uuid::Uuid::from_u128(2)));
    let mut db_cx2 = argentum_core::db::db(&cx_t2);
    let rows2 = PostResource::query(&cx_t2).exec(&mut db_cx2).await.unwrap();
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0].title, "T2 Post");
}

#[tokio::test]
async fn tenantless_requests_to_gated_resources_fail_closed() {
    // GH #87: Author/Post declare requires_tenant — every handler 403s
    // without a tenant instead of leaking rows or minting nil orphans.
    use http::Method;
    use http::header::{CONTENT_TYPE, COOKIE};
    use showcase::models::{Author, seed, seed_phase2};
    use topcoat::router::Body;

    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    seed(&mut db).await.unwrap();
    seed_phase2(&mut db).await.unwrap();
    let router = router(db.clone());

    // List without tenant → 403 (not unscoped rows).
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), 403, "tenantless list must fail closed");

    // Create without tenant → 403 and no row (not a nil-tenant orphan).
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let before = showcase::models::Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .len();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(format!(
                    "title=Orphan&author_id={}&image_path=/tmp/o.jpg&tags=o&csrf_token={csrf}",
                    authors[0].id
                )))
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), 403, "tenantless create must fail closed");
    let after = showcase::models::Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .len();
    assert_eq!(before, after, "no nil-tenant orphan may be minted");
    // Seeds themselves carry the demo tenant — no nil rows exist.
    let nil_rows = showcase::models::Post::filter(
        showcase::models::Post::fields()
            .tenant_id()
            .eq(uuid::Uuid::nil()),
    )
    .exec(&mut db_q)
    .await
    .unwrap();
    assert!(
        nil_rows.is_empty(),
        "seed migration must leave zero nil-tenant rows"
    );
}

#[tokio::test]
async fn header_create_assigns_header_tenant() {
    // GH #87: creates land in the request tenant, never nil.
    use http::Method;
    use http::header::{CONTENT_TYPE, COOKIE};
    use showcase::models::{Author, seed, seed_phase2};
    use topcoat::router::Body;

    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    seed(&mut db).await.unwrap();
    seed_phase2(&mut db).await.unwrap();
    let router = router(db.clone());
    let tenant = showcase::models::DEMO_TENANT;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .header("x-tenant-id", tenant.to_string())
                .body(Body::from(format!(
                    "title=Tenanted&author_id={}&image_path=/tmp/t.jpg&tags=t&csrf_token={csrf}",
                    authors[0].id
                )))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "header-authed create must redirect, got {}",
        resp.status()
    );
    let created = showcase::models::Post::filter(
        showcase::models::Post::fields()
            .title()
            .eq("Tenanted".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("created post");
    assert_eq!(created.tenant_id, tenant);
}
