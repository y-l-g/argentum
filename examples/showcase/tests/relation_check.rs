use http::header::LOCATION;
use showcase::{
    app::router_for_tests as router,
    models::{Author, Comment, Post},
};

use crate::common::{
    assert_hydrate_keys_are_form_fields, body_string, demo_client, full_db, post_count,
};

#[tokio::test]
async fn posts_list_shows_author_name() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts").await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(html.contains("Hello Toasty"), "missing post title {}", html);
    assert!(html.contains("Ada Author"), "missing author name {}", html);
}

#[tokio::test]
async fn posts_create_shows_select_with_author_options() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(html.contains("<select"), "missing select {}", html);
    assert!(
        html.contains("Ada Author"),
        "missing author option {}",
        html
    );
}

#[tokio::test]
async fn posts_create_empty_author_shows_required_error() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = post_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!("title=Test+Post&author_id=&image_path=/tmp/a.jpg&tags=a&csrf_token={csrf}",),
        )
        .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "empty should be 200 not redirect, got {} {}",
        status,
        html
    );
    assert!(
        html.contains("is required") || html.contains("required"),
        "missing required error {}",
        html
    );
    assert_eq!(
        post_count(&db).await,
        before,
        "an invalid create must not add a post"
    );
}

#[tokio::test]
async fn posts_create_invalid_author_shows_invalid_error() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = post_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let fake_id = uuid::Uuid::new_v4();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Test+Post&author_id={}&image_path=/tmp/a.jpg&tags=a&csrf_token={csrf}",
                fake_id
            ),
        )
        .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(status.is_success(), "invalid should be 200 {}", html);
    assert!(
        html.contains("is invalid") || html.contains("invalid"),
        "missing invalid error {}",
        html
    );
    assert_eq!(
        post_count(&db).await,
        before,
        "an invalid create must not add a post"
    );
}

#[tokio::test]
async fn posts_create_valid_redirects_and_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=New+Post&author_id={}&image_path=/tmp/new.jpg&tags=new&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid should redirect, got {} ",
        resp.status()
    );
    let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(loc.contains("/admin/posts"));
    let mut db2 = db.clone();
    let after = Post::all().exec(&mut db2).await.unwrap().len();
    assert_eq!(after, before + 1);
    let created = Post::filter(Post::fields().title().eq("New Post".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap();
    assert!(created.is_some());
}

#[tokio::test]
async fn posts_edit_hydrates_author() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // create a post via valid route to ensure edit hydrates
    let _ = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=EditMe&author_id={}&image_path=/tmp/edit.jpg&tags=edit&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    let mut db2 = db.clone();
    let post = Post::filter(Post::fields().title().eq("EditMe".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .unwrap();
    let edit_url = format!("/admin/posts/{}/edit", post.id);
    let resp = client.get(&edit_url).await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(html.contains("EditMe"), "edit should show title {}", html);
    // GH #108: the hydrated FK must match the option's canonical PK value and
    // be preselected — asserting the id appears is not enough (the option
    // value itself contains it even when nothing is selected).
    let author_option = html
        .split("<option")
        .find(|chunk| chunk.contains(&format!("value=\"{}\"", first.id)))
        .unwrap_or_else(|| panic!("edit should render an option for the stored author {html}"));
    assert!(
        author_option.contains("selected"),
        "the stored author must be preselected: {author_option}"
    );
}

#[tokio::test]
async fn posts_list_shows_comments_count_via_include() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    // GH #217: the expected counts are read from the fixture rather than
    // written as literals, so the assertion names which post gets which count
    // instead of hard-coding the seed's two numbers. The column's *cells* are
    // the observable here; "Comments" alone is the sidebar nav label present on
    // every panel page (GH #216).
    let mut db_q = db.clone();
    let comments_of = async |db: &mut toasty::Db, title: &str| {
        let post = Post::filter(Post::fields().title().eq(title.to_string()))
            .first()
            .exec(db)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the seed creates {title}"));
        Comment::filter(Comment::fields().post_id().eq(post.id))
            .exec(db)
            .await
            .unwrap()
            .len()
    };
    let hello = comments_of(&mut db_q, "Hello Toasty").await;
    let bare = comments_of(&mut db_q, "Second Post").await;
    assert!(
        html.contains(&format!(">{hello}<")),
        "the Comments column must show {hello} for Hello Toasty in {html}"
    );
    assert!(
        html.contains(&format!(">{bare}<")),
        "the Comments column must show {bare} for Second Post in {html}"
    );
    // GH #101: loaded relations must never render the unloaded marker.
    assert!(
        !html.contains("(unloaded)"),
        "unloaded marker leaked into list {}",
        html
    );
}

#[tokio::test]
async fn posts_update_rechecks_author_existence() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let first = &authors[0];
    let posts = Post::all().exec(&mut db_q).await.unwrap();
    let post = &posts[0];
    let edit_url = format!("/admin/posts/{}/edit", post.id);
    // Valid same-author update still redirects (symmetric double-check, GH #91).
    let resp = client
        .csrf(&csrf)
        .post_form(
            &edit_url,
            format!(
                "title=Updated+Title&author_id={}&image_path=/tmp/u.jpg&tags=u&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid update should redirect, got {}",
        resp.status()
    );
    // Bogus author is rejected, not silently written (validate_async invalid).
    let fake = uuid::Uuid::new_v4();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &edit_url,
            format!("title=Bad&author_id={fake}&image_path=/tmp/u.jpg&tags=u&csrf_token={csrf}"),
        )
        .await;
    assert!(
        !resp.status().is_redirection(),
        "bogus author update must not redirect, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn hydrate_form_values_match_schema_fields() {
    use showcase::app::{AuthorResource, PostResource};

    let db = full_db().await;
    let cx = topcoat::context::CxTestBuilder::new()
        .app_context(db.clone())
        .build();
    let mut db_q = db.clone();
    let author = Author::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_hydrate_keys_are_form_fields::<AuthorResource>(&cx, &author);
    let post = Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_hydrate_keys_are_form_fields::<PostResource>(&cx, &post);
}

#[tokio::test]
async fn posts_create_lifecycle_fields_persist() {
    // The full post form: body prose plus static lifecycle selects alongside
    // the author relationship select.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Lifecycle+Post&body=Full+story&status=published&featured=true&author_id={}&image_path=/tmp/life.jpg&tags=life&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "lifecycle POST must redirect, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let created = Post::filter(Post::fields().title().eq("Lifecycle Post".to_string()))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap()
        .expect("lifecycle post");
    assert_eq!(created.body, "Full story");
    assert_eq!(created.status, "published");
    assert!(created.featured);
}

#[tokio::test]
async fn posts_create_omitted_lifecycle_fields_default_to_draft() {
    // Optional-with-defaults: lifecycle fields omitted from the payload
    // create a plain draft, not a validation error.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Stub+Post&author_id={}&image_path=/tmp/stub.jpg&tags=stub&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "stub POST must redirect, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let created = Post::filter(Post::fields().title().eq("Stub Post".to_string()))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap()
        .expect("stub post");
    assert_eq!(created.body, "");
    assert_eq!(created.status, "draft");
    assert!(!created.featured);
}

#[tokio::test]
async fn post_author_options_are_tenant_scoped() {
    // Relationship loads funnel through the tenant-scoped query: a foreign
    // tenant sees none of this tenant's writers.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get("/admin/posts/options?field=author_id").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("Ada Author"),
        "own-tenant options must list writers: {html}"
    );

    let foreign = client
        .tenant(uuid::Uuid::from_u128(4242))
        .get("/admin/posts/options?field=author_id")
        .await;
    assert!(foreign.status().is_success());
    let html = body_string(foreign).await;
    assert!(
        !html.contains("Ada Author"),
        "foreign tenant must not see writers: {html}"
    );
}

#[tokio::test]
async fn post_author_options_deny_blocked_tenant() {
    // Policy denial fails the options load closed: no options, no leak.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client
        .tenant(showcase::models::BLOCKED_TENANT)
        .get("/admin/posts/options?field=author_id")
        .await;
    assert_eq!(
        resp.status(),
        403,
        "blocked tenant options must be forbidden, got {}",
        resp.status()
    );
}
