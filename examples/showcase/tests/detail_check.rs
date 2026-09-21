//! Detail page (GH #187): `GET /admin/{slug}/{id}`, read-only.
//!
//! The page's own rendering is pinned in `argentum-core`'s unit tests (the
//! schema walk and the read-only field shapes); this module pins the HTTP
//! contract — routing, policy, and the answers a bad id gets.

use showcase::{
    app::router_for_tests as router,
    models::{Author, DEMO_ADMIN_PASSWORD, Post, TENANTLESS_ADMIN_EMAIL},
};

use crate::common::{body_string, demo_client, full_db, login, tenanted_db};

/// The first seeded post's id, as the URL carries it.
async fn a_post_id(db: &mut toasty::Db) -> String {
    Post::all()
        .order_by(Post::fields().title().asc())
        .exec(db)
        .await
        .unwrap()
        .remove(0)
        .id
        .to_string()
}

#[tokio::test]
async fn post_detail_renders_the_record_read_only() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    let id = a_post_id(&mut db_q).await;
    let post = Post::all()
        .filter(Post::fields().id().eq(uuid::Uuid::parse_str(&id).unwrap()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the id came from this database");

    let resp = client.get(&format!("/admin/posts/{id}")).await;
    assert!(
        resp.status().is_success(),
        "detail page must render, got {}",
        resp.status()
    );
    let html = body_string(resp).await;

    // The record's own values, rendered as text.
    assert!(
        html.contains(&post.title),
        "detail page must show the title: {html}"
    );
    assert!(
        html.contains(&post.status),
        "detail page must show the status: {html}"
    );
    assert!(
        html.contains("Back to list"),
        "detail page must offer a way back: {html}"
    );

    // Read-only means read-only. The shell carries its own chrome (the sign-out
    // form), so the claim is scoped to the page body: everything from the page
    // heading to the end of `main`.
    let body = html
        .split("<h1")
        .nth(1)
        .and_then(|rest| rest.split("</main>").next())
        .expect("the detail page renders inside the shell's main");
    assert!(
        !body.contains("<input") && !body.contains("<select") && !body.contains("<form"),
        "a detail page must not render form controls: {body}"
    );
    assert!(
        !body.contains("text-destructive") && !body.contains("ac-field--error"),
        "a stored record has nothing to be invalid about: {body}"
    );
    // A field is present as a value, not as a control: the read-only shape.
    assert!(
        body.contains("whitespace-pre-wrap"),
        "the page renders values through the read-only field shape: {body}"
    );
}

#[tokio::test]
async fn post_detail_is_scoped_like_every_other_route() {
    // Unknown id and wrong tenant are one answer (ADR-0002): the load runs
    // through `Resource::query`, so the page cannot tell the caller which ids
    // exist outside their scope.
    let (db, t1, t2) = tenanted_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    let post = Post::all()
        .filter(Post::fields().tenant_id().eq(t1))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the fixture seeds posts for t1");

    let unknown = client
        .get("/admin/posts/00000000-0000-0000-0000-000000000000")
        .await;
    assert_eq!(unknown.status(), 404, "an unknown id is not found");

    let other_tenant = client
        .tenant(t2)
        .get(&format!("/admin/posts/{}", post.id))
        .await;
    assert_eq!(
        other_tenant.status(),
        404,
        "a record outside the request's scope looks exactly like an unknown id"
    );

    let own_tenant = client
        .tenant(t1)
        .get(&format!("/admin/posts/{}", post.id))
        .await;
    assert!(
        own_tenant.status().is_success(),
        "the owning tenant reaches its own record, got {}",
        own_tenant.status()
    );
}

#[tokio::test]
async fn resources_without_a_view_declaration_have_no_detail_page() {
    // `AuthorResource` declares no `view`, so the route answers as if there
    // were none — which is what makes the missing row link honest.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    let author = Author::all().exec(&mut db_q).await.unwrap().remove(0);

    let resp = client.get(&format!("/admin/authors/{}", author.id)).await;
    assert_eq!(
        resp.status(),
        404,
        "a resource that declares no view has no detail page"
    );

    // …and the list offers no View link for it, while the posts list does.
    let authors = body_string(client.get("/admin/authors").await).await;
    assert!(
        !authors.contains(">View<"),
        "no view declaration means no View link: {authors}"
    );
    let posts = body_string(client.get("/admin/posts").await).await;
    assert!(
        posts.contains(">View<"),
        "a declared view means a View link per row: {posts}"
    );
    // The link carries a *record* key (GH #168), so the href is the seeded
    // post's own id and not a display key.
    let mut db_q = db;
    let post_id = a_post_id(&mut db_q).await;
    assert!(
        posts.contains(&format!("href=\"/admin/posts/{post_id}\"")),
        "the View link points at the detail route with the record key: {posts}"
    );
}

#[tokio::test]
async fn the_detail_route_does_not_shadow_create_or_edit() {
    // `/{slug}/{id}` shares its segment position with the literal `create`
    // route and prefixes `/{id}/edit`: the router prefers the static segment
    // and the longer path, so neither page is lost to the detail route.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db;
    let id = a_post_id(&mut db_q).await;

    let create = client.get("/admin/posts/create").await;
    assert!(
        create.status().is_success(),
        "create page must still render, got {}",
        create.status()
    );
    let create_html = body_string(create).await;
    assert!(
        create_html.contains("<form") && create_html.contains("csrf_token"),
        "the create page must still be the form: {create_html}"
    );

    let edit = client.get(&format!("/admin/posts/{id}/edit")).await;
    assert!(
        edit.status().is_success(),
        "edit page must still render, got {}",
        edit.status()
    );
    let edit_html = body_string(edit).await;
    assert!(
        edit_html.contains("<form") && edit_html.contains("csrf_token"),
        "the edit page must still be the form: {edit_html}"
    );
}

#[tokio::test]
async fn post_detail_enforces_requires_tenant() {
    // `PostResource::requires_tenant` is true (GH #87/#131): a signed-in user
    // with no tenant must be refused here too, not shown an unscoped record —
    // even for an id that exists.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let id = a_post_id(&mut db_q).await;
    let client = login(&router, TENANTLESS_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    let resp = client.get(&format!("/admin/posts/{id}")).await;
    assert_eq!(
        resp.status(),
        403,
        "a tenant-gated resource must fail closed on its detail page"
    );
}

#[tokio::test]
async fn post_detail_hides_the_record_from_a_denied_tenant() {
    // The blocked tenant is refused before policy is even consulted: its
    // `query` filter finds no such row, so the answer is the same 404 an
    // unknown id gets — scoping first, `can_view` second, which is the order
    // that keeps a 403 from confirming a record's existence across tenants.
    use showcase::models::BLOCKED_TENANT;

    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    // A real id, so the refusal comes from the policy and not from the load.
    let id = a_post_id(&mut db_q).await;

    let resp = client
        .tenant(BLOCKED_TENANT)
        .get(&format!("/admin/posts/{id}"))
        .await;
    assert_eq!(
        resp.status(),
        404,
        "a tenant the query scopes out sees the same answer as an unknown id"
    );
}
