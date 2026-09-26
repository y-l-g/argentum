use showcase::{
    app::router_for_tests as router,
    models::{Author, Post},
};

use crate::common::{body_string, demo_client, full_db, post_count};

#[tokio::test]
async fn posts_create_shows_cover_picker_and_repeater() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    // One media source: the cover is a picked library row, not a file input.
    assert!(
        !html.contains("type=\"file\""),
        "the post form must not upload a cover directly: {html}"
    );
    assert!(
        html.contains("name=\"cover_id\""),
        "missing cover picker {html}"
    );
    assert!(
        html.contains("data-slot=\"field\""),
        "missing field wrapper {html}"
    );
    // Repeater should render nested schema with Tags label and inner Tag input
    assert!(html.contains("Tags"), "missing Repeater label {html}");
    assert!(
        html.contains("for=\"tags\"") || html.contains("name=\"tags\""),
        "missing tags input {html}"
    );
    // Content/Group composition: sectioned story fields and a grouped metadata
    // grid.
    assert!(html.contains("Content"), "missing Content section {html}");
    assert!(
        html.contains("name=\"status\"") && html.contains("name=\"featured\""),
        "missing lifecycle selects {html}"
    );
    // The form's own labels: the flag select reads "Featured" and the cover
    // picker reads "Cover".
    assert!(
        html.contains("Featured</label>"),
        "missing Featured label for the flag select {html}"
    );
    assert!(
        html.contains(">Cover<") || html.contains("name=\"cover_id\""),
        "missing Cover picker {html}"
    );
    assert!(
        html.contains("field-group"),
        "missing Group container {html}"
    );
    assert!(
        html.contains("grid grid-cols-2") || html.contains("grid-cols-2"),
        "missing Grid {html}"
    );
}

#[tokio::test]
async fn posts_create_invalid_repeater_shows_errors() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = post_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // Missing title (required). The optional Tags group is empty, which is
    // absent — not an error.
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!("title=&author_id={}&tags=&csrf_token={csrf}", first.id),
        )
        .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "invalid should be 200, got {status} {html}"
    );
    assert!(
        html.contains("Title is required"),
        "missing required error for the title field, got {html}"
    );
    assert_eq!(
        post_count(&db).await,
        before,
        "an invalid create must not add a post"
    );
}

#[tokio::test]
async fn posts_create_valid_repeater_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let author_id = first.id.to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Valid+With+Tags&author_id={author_id}&cover_id=&tags=valid%2Ctags&csrf_token={csrf}"
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid should redirect, got {} ",
        resp.status()
    );
    let mut db2 = db.clone();
    let after = Post::all().exec(&mut db2).await.unwrap().len();
    assert_eq!(after, before + 1);
    let created = Post::filter(Post::fields().title().eq("Valid With Tags".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap();
    assert!(created.is_some());
    let post = created.unwrap();
    assert_eq!(post.tags, "valid,tags");
    assert_eq!(post.cover_id, None);
}

/// An optional Repeater with a `required` inner input must not fail an empty
/// submit: group-empty means "absent". The shipped `/admin/posts`
/// form is exactly that shape (optional `Tags` over a required inner input),
/// so an empty Tags group submits cleanly. A partially filled group still
/// enforces inner `required` — pinned at the schema level, where the shape is
/// expressible (a single-entry repeater submits one entry, so the values
/// cannot distinguish "group absent" from "group present" in this form).
#[tokio::test]
async fn posts_create_with_empty_optional_tags_group_submits() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let before = Post::all().exec(&mut db2).await.unwrap().len();

    let author_id = authors[0].id.to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!("title=No+Tags&author_id={author_id}&cover_id=&tags=&csrf_token={csrf}"),
        )
        .await;
    let status = resp.status();
    assert!(
        status.is_redirection(),
        "an empty optional Tags group must not fail the submit, got {status} {}",
        body_string(resp).await
    );
    let mut db2 = db.clone();
    assert_eq!(
        Post::all().exec(&mut db2).await.unwrap().len(),
        before + 1,
        "the post is created"
    );
    let created = Post::filter(Post::fields().title().eq("No Tags".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("created post");
    assert_eq!(created.tags, "", "the empty group stores empty");
}

#[tokio::test]
async fn users_create_form_stays_urlencoded() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/users/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        !html.contains("multipart/form-data"),
        "plain form must stay urlencoded, got {}",
        &html[..html.len().min(2000)]
    );
}

#[tokio::test]
async fn posts_create_form_stays_urlencoded_without_uploads() {
    // One media source: the post form picks a library row instead of uploading
    // bytes, so it stays urlencoded like every other plain form.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        !html.contains("multipart/form-data"),
        "the post form carries no file input, got {}",
        &html[..html.len().min(2000)]
    );
}

#[tokio::test]
async fn posts_author_select_is_searchable() {
    // GH #91: the relationship select carries the client-side filter hook.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("data-options-filter"),
        "author select must render the filter hook, got {html}"
    );
    // GH #236: hiding the native select is the script's job, so the markup keeps
    // both controls. The select stays the submitted value carrier, and
    // `partsOf` keeps resolving it as a descendant of the filterable field.
    let author_select = html
        .match_indices("<select")
        .map(|(start, _)| opening_tag_at(&html, start))
        .find(|tag| tag.contains("name=\"author_id\""))
        .expect("the author select stays in the markup as the submitted value carrier");
    assert!(
        author_select.contains("id=\"author_id\""),
        "the author select keeps its field id, got {author_select}"
    );
}

/// The opening tag that starts at `start`, up to its unquoted `>`.
fn opening_tag_at(html: &str, start: usize) -> &str {
    let mut quoted = false;
    for (offset, byte) in html[start..].bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'>' if !quoted => return &html[start..start + offset],
            _ => {}
        }
    }
    panic!("unterminated tag at byte {start}");
}
