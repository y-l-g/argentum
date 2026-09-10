use http::header::COOKIE;
use showcase::{
    app::router_for_tests as router,
    models::{
        Author, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, DEMO_TENANT, Post, TENANTLESS_ADMIN_EMAIL,
    },
};
use topcoat::router::Body;

mod common;
use common::{
    SESSION_COOKIE, body_string, demo_client, full_db, login, login_next, session_cookie_value,
    tenanted_db,
};

#[tokio::test]
async fn logged_in_tenant_reaches_tenant_scoped_resources_without_headers() {
    // GH #131: the demo admin's tenant flows from the login, so tenant-scoped
    // resources serve without any tenant header or request extension.
    let db = full_db().await;
    let router = router(db);
    let client = login(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    for path in ["/admin/authors", "/admin/posts"] {
        let response = client.get(path).await;
        assert_eq!(response.status(), 200, "{path}");
    }
    let html = body_string(client.get("/admin/authors").await).await;
    assert!(html.contains("Ada Author"), "{html}");
}

#[tokio::test]
async fn posts_list_is_scoped_by_tenant_via_resource_query() {
    let (db, t1, t2) = tenanted_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    let resp_t1 = client.tenant(t1).get("/admin/posts").await;
    assert!(resp_t1.status().is_success());
    let html = body_string(resp_t1).await;
    assert!(html.contains("T1 Post"), "t1 should see T1 Post {}", html);
    assert!(
        !html.contains("T2 Post"),
        "t1 should not see T2 Post {}",
        html
    );

    let resp_t2 = client.tenant(t2).get("/admin/posts").await;
    assert!(resp_t2.status().is_success());
    let html = body_string(resp_t2).await;
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
    let client = demo_client(&router).await;
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
    let resp = client.tenant(t2).get(&edit_url).await;
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
    let client = demo_client(&router).await;
    let blocked = uuid::Uuid::from_u128(9999);
    let resp = client.tenant(blocked).get("/admin/posts").await;
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
    // GH #87/#131: Author/Post declare requires_tenant — every handler 403s
    // when the logged-in user carries no tenant, instead of leaking rows or
    // minting nil orphans.
    let db = full_db().await;
    let router = router(db.clone());
    let client = login(&router, TENANTLESS_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    // List without a tenant → 403 (not unscoped rows).
    let resp = client.get("/admin/posts").await;
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
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Orphan&author_id={}&image_path=/tmp/o.jpg&tags=o&csrf_token={csrf}",
                authors[0].id
            ),
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
async fn create_assigns_the_logged_in_tenant() {
    // GH #87/#131: creates land in the tenant the logged-in user carries,
    // never nil.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let tenant = DEMO_TENANT;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Tenanted&author_id={}&image_path=/tmp/t.jpg&tags=t&csrf_token={csrf}",
                authors[0].id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "authenticated create must redirect, got {}",
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

#[tokio::test]
async fn x_tenant_id_header_no_longer_grants_a_tenant() {
    // GH #131: learning another tenant's UUID must not make the caller that
    // tenant through the old harness header.
    let db = full_db().await;
    let router = router(db);
    let (_, login_response) =
        login_next(&router, TENANTLESS_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, "").await;
    let session = session_cookie_value(&login_response).expect("session cookie");
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/posts")
                .header(COOKIE, format!("{SESSION_COOKIE}={session}"))
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        response.status(),
        403,
        "x-tenant-id must not grant a tenant (GH #131)"
    );
}
