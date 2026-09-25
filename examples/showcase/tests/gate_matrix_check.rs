//! The route gate matrix: every panel route's auth, CSRF, tenant and
//! policy gates restated as a request-level assertion.
//!
//! The handlers apply every gate; the point of this module is that each one is
//! pinned at the route it protects, not inferred from another route's test. The
//! CSRF rows cover the routes no other suite posts a forged token to (bulk
//! delete, multipart create/edit, login, logout), the cross-tenant rows send a
//! *valid* token from the wrong tenant, and the policy/anonymity rows enumerate
//! the read and mutation shapes. A refactor that drops one of these gates fails
//! here instead of passing the whole suite.

use http::header::LOCATION;
use showcase::{
    app::router_for_tests as router,
    models::{
        Author, BLOCKED_TENANT, Comment, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, Media, Post,
        PostStats, Publication, Seo,
    },
};
use uuid::Uuid;

use crate::common::{
    SESSION_COOKIE, TestClient, comment_count, demo_client, form_body, full_db, mint_session,
    post_count, session_cookie_value, tenanted_db, user_count,
};

/// A session-holding POST with **no** CSRF cookie and no `csrf_token` field is
/// refused: the double-submit check needs both halves, so an absent
/// cookie must not read as "nothing to compare" and let the write through.
///
/// The forged rows above always present a CSRF cookie; this pins the
/// conjunction on the url-encoded create route their multipart rows do not
/// cover.
#[tokio::test]
async fn a_csrf_cookie_and_field_are_both_required() {
    let db = full_db().await;
    let router = router(db.clone());
    // A real session, so the auth gate passes and the CSRF check is the only
    // gate that can refuse the request.
    let session = mint_session(&db, DEMO_ADMIN_EMAIL).await;
    let client = TestClient::new(&router).cookie(SESSION_COOKIE, &session);
    let before = user_count(&db).await;

    let resp = client
        .post_form(
            "/admin/users/create",
            "name=NoToken&email=notoken%40example.com".to_string(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "a POST with no CSRF cookie and no field must 403, got {}",
        resp.status()
    );
    assert_eq!(
        user_count(&db).await,
        before,
        "a refused CSRF check must create nothing"
    );
}

/// A forged CSRF pair is refused on the post routes whose rejection no other
/// suite pins — url-encoded delete and bulk delete, plus the multipart
/// create/edit upload path — and nothing they name changes.
///
/// Each route is sent twice: a field token that differs from the cookie, and no
/// `csrf_token` field at all. Both must answer 403. The regression it catches
/// is a handler that verifies CSRF after its DB work (or not at all): the
/// mismatch would then create, update or delete a row before failing.
#[tokio::test]
async fn forged_posts_answer_403_and_change_nothing() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let mut db_q = db.clone();
    let post = Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a seeded post");
    let author = Author::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a seeded author");

    let field = Uuid::new_v4().to_string();
    let cookie = Uuid::new_v4().to_string();
    let before = post_count(&db).await;

    // 1. Row delete, url-encoded.
    for (body, label) in [
        (format!("confirm=1&csrf_token={field}"), "mismatched token"),
        ("confirm=1".to_string(), "missing token"),
    ] {
        let resp = client
            .csrf(&cookie)
            .post_form(&format!("/admin/posts/{}/delete", post.id), body)
            .await;
        assert_eq!(
            resp.status(),
            403,
            "delete {label}: a forged POST must 403, got {}",
            resp.status()
        );
    }
    assert_eq!(
        post_count(&db).await,
        before,
        "a forged delete must remove nothing"
    );

    // 2. Bulk delete, url-encoded.
    for (body, label) in [
        (
            format!("ids={}&confirm=1&csrf_token={field}", post.id),
            "mismatched token",
        ),
        (format!("ids={}&confirm=1", post.id), "missing token"),
    ] {
        let resp = client
            .csrf(&cookie)
            .post_form("/admin/posts/bulk-delete", body)
            .await;
        assert_eq!(
            resp.status(),
            403,
            "bulk delete {label}: a forged POST must 403, got {}",
            resp.status()
        );
    }
    assert_eq!(
        post_count(&db).await,
        before,
        "a forged bulk delete must remove nothing"
    );

    // 3./4. Multipart create and edit: the upload path shares the CSRF verify.
    let boundary = "----GateMatrixBoundary";
    for (csrf_part, label) in [
        (
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{field}\r\n"
            ),
            "mismatched token",
        ),
        (String::new(), "missing token"),
    ] {
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nForged\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"author_id\"\r\n\r\n{author_id}\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"x.png\"\r\nContent-Type: image/png\r\n\r\nFORGED\r\n\
             {csrf_part}--{b}--\r\n",
            b = boundary,
            author_id = author.id,
        );
        let resp = client
            .csrf(&cookie)
            .post_multipart("/admin/posts/create", boundary, body)
            .await;
        assert_eq!(
            resp.status(),
            403,
            "multipart create {label}: a forged POST must 403, got {}",
            resp.status()
        );

        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nForged\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"author_id\"\r\n\r\n{author_id}\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"x.png\"\r\nContent-Type: image/png\r\n\r\nFORGED\r\n\
             {csrf_part}--{b}--\r\n",
            b = boundary,
            author_id = author.id,
        );
        let resp = client
            .csrf(&cookie)
            .post_multipart(&format!("/admin/posts/{}/edit", post.id), boundary, body)
            .await;
        assert_eq!(
            resp.status(),
            403,
            "multipart edit {label}: a forged POST must 403, got {}",
            resp.status()
        );
    }
    assert_eq!(
        post_count(&db).await,
        before,
        "a forged multipart create must write nothing"
    );
    let unchanged = Post::filter(Post::fields().id().eq(post.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        unchanged.title, post.title,
        "a forged multipart edit must not rewrite the title"
    );
}

/// A forged login POST is refused before any credential work and mints no
/// session.
///
/// The CSRF verify runs before the password is read, so a mismatched token and
/// a missing `csrf_token` field are both 403 rather than a credential verdict,
/// and a rejected attempt sets no session cookie. The regression it catches is
/// a login handler that authenticates first and verifies the token (if at all)
/// afterwards.
#[tokio::test]
async fn forged_login_answers_403_and_sets_no_session() {
    let db = full_db().await;
    let router = router(db.clone());
    let field = Uuid::new_v4().to_string();
    let cookie = Uuid::new_v4().to_string();

    for (submitted, label) in [
        (Some(field.as_str()), "mismatched token"),
        (None, "missing token"),
    ] {
        let mut pairs = vec![
            ("email", DEMO_ADMIN_EMAIL),
            ("password", DEMO_ADMIN_PASSWORD),
        ];
        if let Some(field) = submitted {
            pairs.push(("csrf_token", field));
        }
        let resp = TestClient::new(&router)
            .csrf(&cookie)
            .post_form("/admin/login", form_body(&pairs))
            .await;
        assert_eq!(
            resp.status(),
            403,
            "login {label}: a forged POST must 403, got {}",
            resp.status()
        );
        assert!(
            session_cookie_value(&resp).is_none(),
            "login {label}: a forged login must set no session cookie"
        );
    }
}

/// A forged logout POST is refused and leaves the session usable.
///
/// Both a mismatched token and a missing `csrf_token` field must 403. The
/// regression it catches is a logout that deletes the session row before
/// verifying the token, or not at all: the presented session must survive the
/// forged pair, which the follow-up authenticated GET proves.
#[tokio::test]
async fn forged_logout_answers_403_and_keeps_the_session() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let field = Uuid::new_v4().to_string();
    let cookie = Uuid::new_v4().to_string();

    for (submitted, label) in [
        (Some(field.as_str()), "mismatched token"),
        (None, "missing token"),
    ] {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(field) = submitted {
            pairs.push(("csrf_token", field));
        }
        let resp = client
            .csrf(&cookie)
            .post_form("/admin/logout", form_body(&pairs))
            .await;
        assert_eq!(
            resp.status(),
            403,
            "logout {label}: a forged POST must 403, got {}",
            resp.status()
        );
    }
    assert_eq!(
        client.get("/admin/posts").await.status(),
        200,
        "a forged logout must leave the session usable"
    );
}

/// A valid CSRF pair does not buy a cross-tenant write: tenant B's
/// edit and delete of tenant A's post and comment 404 through the tenant-scoped
/// load, and the rows are untouched.
///
/// `delete_404_for_an_unknown_id` pins the unknown-id half with a random UUID;
/// this pins the wrong-tenant half with the token the browser would actually
/// send. The edit POST 404s on its advisory, tenant-scoped load before it opens
/// a transaction (`resource_edit_post`), so the in-transaction reload that
/// repeats the scope is a second seam this test does not reach; the delete POST
/// runs its scoped load inside the transaction. The owner's own delete of the
/// same comment redirects, proving the 404s are the scope and not a dead route.
#[tokio::test]
async fn cross_tenant_edit_and_delete_404_and_touch_nothing() {
    let (db, t1, t2) = tenanted_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let comments_before = comment_count(&db).await;

    let t1_post = Post::filter(Post::fields().tenant_id().eq(t1))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("t1 post");
    let t1_comment = Comment::filter(Comment::fields().body().eq("T1 comment".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("t1 comment");
    let csrf = Uuid::new_v4().to_string();
    let foreign = client.tenant(t2).csrf(&csrf);
    let author_id = t1_post.author_id.to_string();

    let resp = foreign
        .post_form(
            &format!("/admin/posts/{}/edit", t1_post.id),
            form_body(&[
                ("title", "Hijacked"),
                ("author_id", &author_id),
                ("csrf_token", &csrf),
            ]),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "a cross-tenant post edit must 404, got {}",
        resp.status()
    );

    let resp = foreign
        .post_form(
            &format!("/admin/posts/{}/delete", t1_post.id),
            form_body(&[("confirm", "1"), ("csrf_token", &csrf)]),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "a cross-tenant post delete must 404, got {}",
        resp.status()
    );

    let surviving_post = Post::filter(Post::fields().id().eq(t1_post.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the t1 post still exists");
    assert_eq!(
        surviving_post.title, t1_post.title,
        "a cross-tenant edit must not rewrite the title"
    );

    // Comments inherit their post's tenant, so the same pair 404s there too.
    let post_id = t1_post.id.to_string();
    let resp = foreign
        .post_form(
            &format!("/admin/comments/{}/edit", t1_comment.id),
            form_body(&[
                ("body", "Hijacked"),
                ("post_id", &post_id),
                ("csrf_token", &csrf),
            ]),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "a cross-tenant comment edit must 404, got {}",
        resp.status()
    );

    let resp = foreign
        .post_form(
            &format!("/admin/comments/{}/delete", t1_comment.id),
            form_body(&[("confirm", "1"), ("csrf_token", &csrf)]),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "a cross-tenant comment delete must 404, got {}",
        resp.status()
    );

    let surviving_comment = Comment::filter(Comment::fields().id().eq(t1_comment.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the t1 comment still exists");
    assert_eq!(
        surviving_comment.body, t1_comment.body,
        "a cross-tenant comment edit must not rewrite the body"
    );
    assert_eq!(
        comment_count(&db).await,
        comments_before,
        "a cross-tenant comment delete must remove nothing"
    );

    let resp = client
        .tenant(t1)
        .csrf(&csrf)
        .post_form(
            &format!("/admin/comments/{}/delete", t1_comment.id),
            form_body(&[("confirm", "1"), ("csrf_token", &csrf)]),
        )
        .await;
    assert_eq!(
        resp.status(),
        303,
        "the owner's comment delete must be a 303 PRG, got {}",
        resp.status()
    );
    assert_eq!(
        resp.headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin/comments"),
        "the owner's comment delete must redirect to the comments list"
    );
    let gone = Comment::filter(Comment::fields().id().eq(t1_comment.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap();
    assert!(
        gone.is_none(),
        "the owner's comment delete must remove the row"
    );
}

/// The policy gate refuses `BLOCKED_TENANT` on every read route, not only the
/// list: the list, the CSV export and the detail page each consult
/// `can_view_any`/`can_view`, so a refactor that dropped the policy check from
/// one of them fails here.
///
/// The detail page's check only runs against a loaded record, so the blocked
/// tenant needs a row of its own; otherwise the scoped load 404s first and the
/// policy denial would go untested.
#[tokio::test]
async fn blocked_tenant_is_refused_on_every_read_route() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let blocked = client.tenant(BLOCKED_TENANT);

    let mut db_q = db.clone();
    let blocked_author = toasty::create!(Author {
        tenant_id: BLOCKED_TENANT,
        name: "Blocked Author",
        email: "blocked@example.com",
    })
    .exec(&mut db_q)
    .await
    .expect("create an author under BLOCKED_TENANT");
    let blocked_post = toasty::create!(Post {
        tenant_id: BLOCKED_TENANT,
        title: "Blocked Post",
        body: "body",
        status: "draft".to_string(),
        featured: false,
        created_at: "2024-01-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/blocked.jpg".to_string(),
        tags: "blocked".to_string(),
        seo: Seo {
            title: "Blocked SEO".to_string(),
            description: String::new(),
        },
        publication: Publication::Published {
            published_at: "2024-01-01T00:00:00Z".to_string(),
            canonical_url: String::new(),
        },
        media: Media::Image {
            url: "/images/blocked.jpg".to_string(),
            alt: String::new(),
        },
        post_stats: PostStats {
            word_count: 0,
            read_minutes: 0,
        },
        author_id: blocked_author.id,
    })
    .exec(&mut db_q)
    .await
    .expect("create a post under BLOCKED_TENANT");

    for path in [
        "/admin/posts".to_string(),
        "/admin/posts/export".to_string(),
        format!("/admin/posts/{}", blocked_post.id),
    ] {
        let resp = blocked.get(&path).await;
        assert_eq!(
            resp.status(),
            403,
            "{path}: a blocked tenant must be refused, got {}",
            resp.status()
        );
    }
}

/// Anonymous requests are gated on every route shape: reads redirect
/// to the login page, mutations answer 401 and change nothing.
///
/// `auth_check.rs` asserts the exact `Location` for five panel GETs (four list
/// pages and the create form); this enumerates the panel's record, form, export
/// and option reads plus the mutation routes, with the same exact-`Location` and
/// 401 answers, so a route mounted without the gate is caught here.
#[tokio::test]
async fn anonymous_requests_are_gated_on_every_route() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = TestClient::new(&router);

    let mut db_q = db.clone();
    let post = Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a seeded post");
    let before = post_count(&db).await;

    for path in [
        "/admin/posts".to_string(),
        "/admin/posts/create".to_string(),
        format!("/admin/posts/{}", post.id),
        format!("/admin/posts/{}/edit", post.id),
        "/admin/posts/export".to_string(),
        "/admin/posts/options?q=a".to_string(),
    ] {
        let resp = client.get(&path).await;
        assert_eq!(
            resp.status(),
            307,
            "{path}: an anonymous read must redirect, got {}",
            resp.status()
        );
        let expected = format!("/admin/login?{}", form_body(&[("next", &path)]));
        assert_eq!(
            resp.headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some(expected.as_str()),
            "{path}: the redirect must carry the login route and the validated next"
        );
    }

    for path in [
        "/admin/posts/create".to_string(),
        format!("/admin/posts/{}/edit", post.id),
        format!("/admin/posts/{}/delete", post.id),
        "/admin/posts/bulk-delete".to_string(),
    ] {
        let resp = client.post_form(&path, "confirm=1".to_string()).await;
        assert_eq!(
            resp.status(),
            401,
            "{path}: an anonymous mutation must answer 401, got {}",
            resp.status()
        );
    }
    assert_eq!(
        post_count(&db).await,
        before,
        "anonymous mutations must change nothing"
    );
}
