use showcase::{
    app::router_for_tests as router,
    models::{Author, Post},
};

mod common;
use common::{body_string, demo_client, full_db, tenanted_db};

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
    // GH #87: Author/Post declare requires_tenant — every handler 403s
    // without a tenant instead of leaking rows or minting nil orphans.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    // List without tenant → 403 (not unscoped rows).
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
async fn header_create_assigns_header_tenant() {
    // GH #87: creates land in the request tenant, never nil.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let tenant = showcase::models::DEMO_TENANT;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let resp = client
        .tenant(tenant)
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
