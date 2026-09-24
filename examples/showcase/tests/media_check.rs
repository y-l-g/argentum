//! The media library (GH #248): the app-level upload path writes a `medias` row
//! tied to its polymorphic owner, the stored rows render a thumbnail or a link,
//! and the clear control works with and without JavaScript.
//!
//! The widget's JavaScript half is `examples/showcase/assets/media.test.js`
//! (Node); what a server-rendered page can pin is the markup contract those
//! hooks describe — the clear control is a reset button, so a browser empties
//! the file input with no script at all.

use http_body_util::BodyExt;
use showcase::{
    app::router_with_app_uploads,
    media::{KIND_FILE, KIND_IMAGE, MEDIA_PATH, MediaOwner, OWNER_POST, media_for_owner},
    models::{DEMO_TENANT, MediaAsset, Post, User},
};
use topcoat::router::{Body, Router};

use crate::common::{TestClient, body_string, demo_client, full_db, tenantless_client};

/// Bytes of an uploaded file: ASCII, so the multipart body can be a `String`,
/// which is all the test client takes.
const PAYLOAD: &str = "PNG-FAKE-BYTES";

/// The one published seed post — the media library's post owner.
async fn published_post(db: &toasty::Db) -> Post {
    let mut db = db.clone();
    Post::filter(Post::fields().status().eq("published".to_string()))
        .first()
        .exec(&mut db)
        .await
        .expect("query the published seed post")
        .expect("the seed publishes one post")
}

/// The seeded draft: a second owner of the same kind as [`published_post`].
async fn draft_post(db: &toasty::Db) -> Post {
    let mut db = db.clone();
    Post::filter(Post::fields().title().eq("Second Post".to_string()))
        .first()
        .exec(&mut db)
        .await
        .expect("query the draft seed post")
        .expect("the seed creates the Second Post draft")
}

/// How many media rows the database holds.
async fn media_count(db: &toasty::Db) -> usize {
    let mut db = db.clone();
    MediaAsset::all().exec(&mut db).await.unwrap().len()
}

/// POST one multipart upload as the page's form does, returning the response.
///
/// `csrf` is the token to embed; the matching cookie goes on the client. `None`
/// posts no token at all, which is the forged-request case.
async fn post_upload(
    client: &TestClient<'_>,
    owner: MediaOwner,
    filename: &str,
    content_type: &str,
    payload: &str,
    csrf: Option<&str>,
) -> http::Response<Body> {
    let client = match csrf {
        Some(token) => client.csrf(token),
        None => client.clone(),
    };
    let boundary = "----MediaBoundary";
    let token = csrf
        .map(|token| {
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{token}\r\n"
            )
        })
        .unwrap_or_default();
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"owner\"\r\n\r\n{owner}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n{payload}\r\n\
         {token}--{b}--\r\n",
        b = boundary,
        owner = owner.value(),
    );
    client.post_multipart(MEDIA_PATH, boundary, body).await
}

/// One upload with a freshly minted CSRF pair, as the demo admin.
async fn upload(
    router: &Router,
    db: &toasty::Db,
    owner: MediaOwner,
    filename: &str,
    content_type: &str,
    payload: &str,
) -> http::Response<Body> {
    let client = demo_client(router, db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    post_upload(&client, owner, filename, content_type, payload, Some(&csrf)).await
}

/// The opening tag carrying `needle`.
///
/// Attributes render in no guaranteed order (topcoat#122), so a case locates a
/// tag by whichever attribute it can and asserts on the whole tag. Quoting is
/// honoured, so a `>` inside an attribute value does not end the slice.
fn tag_with<'h>(html: &'h str, needle: &str) -> &'h str {
    let at = html
        .find(needle)
        .unwrap_or_else(|| panic!("no {needle} in {html}"));
    let start = html[..at].rfind('<').expect("its opening tag");
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

/// The upload form's markup, so a case can assert what is inside it.
fn upload_form(html: &str) -> &str {
    let at = html
        .find("enctype=\"multipart/form-data\"")
        .unwrap_or_else(|| panic!("no upload form in {html}"));
    let start = html[..at].rfind("<form").expect("its opening tag");
    let end = html[start..].find("</form>").expect("its closing tag") + start;
    &html[start..end]
}

#[tokio::test]
async fn an_upload_creates_a_row_tied_to_its_owner() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);

    let response = upload(&router, &db, owner, "cover.png", "image/png", PAYLOAD).await;
    assert!(
        response.status().is_redirection(),
        "the upload must save, got {}",
        response.status()
    );

    let mut db_q = db.clone();
    let rows = media_for_owner(&mut db_q, owner)
        .await
        .expect("query the owner's media");
    assert_eq!(rows.len(), 1, "one row for the upload");
    let row = &rows[0];
    assert_eq!(row.owner_type, OWNER_POST);
    assert_eq!(
        row.owner_id, post.id,
        "the row names the post it was uploaded for"
    );
    assert_eq!(row.tenant_id, DEMO_TENANT);
    assert_eq!(row.filename, "cover.png");
    assert_eq!(row.kind, KIND_IMAGE);
    assert!(
        row.path.starts_with("/uploads/"),
        "the row stores the URL the store returned, got {}",
        row.path
    );
    assert!(row.path.ends_with("cover.png"), "got {}", row.path);

    // The bytes are in the directory the panel serves, and the stored path —
    // exactly the string in the row — fetches them back.
    let client = demo_client(&router, &db).await;
    let response = client.get(&row.path).await;
    assert_eq!(response.status(), 200, "{} must be fetchable", row.path);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect the served file")
        .to_bytes();
    assert_eq!(bytes.as_ref(), PAYLOAD.as_bytes());
}

#[tokio::test]
async fn a_lookup_returns_one_owners_rows_and_not_every_row_of_that_kind() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    // Two owners of the same kind: the pair's second half is what tells them
    // apart, so a lookup that dropped `owner_id` would hand one owner the
    // other's rows — the leak ADR-0021 makes the app responsible for.
    let first = published_post(&db).await;
    let second = draft_post(&db).await;
    assert_ne!(first.id, second.id);

    let first_owner = MediaOwner::Post(first.id);
    let second_owner = MediaOwner::Post(second.id);
    upload(&router, &db, first_owner, "first.png", "image/png", PAYLOAD).await;
    upload(
        &router,
        &db,
        second_owner,
        "second.png",
        "image/png",
        PAYLOAD,
    )
    .await;

    let mut db_q = db.clone();
    let on_first = media_for_owner(&mut db_q, first_owner).await.unwrap();
    let mut db_q = db.clone();
    let on_second = media_for_owner(&mut db_q, second_owner).await.unwrap();

    assert_eq!(on_first.len(), 1, "one row per owner, not two");
    assert_eq!(on_second.len(), 1, "one row per owner, not two");
    assert_eq!(on_first[0].owner_id, first.id);
    assert_eq!(on_second[0].owner_id, second.id);
    assert_eq!(on_first[0].filename, "first.png");
    assert_eq!(on_second[0].filename, "second.png");
    assert_ne!(on_first[0].id, on_second[0].id);
}

#[tokio::test]
async fn a_media_row_can_name_either_owner_kind() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let mut db_q = db.clone();
    let user = showcase::models::User::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .remove(0);

    let post_owner = MediaOwner::Post(post.id);
    let user_owner = MediaOwner::User(user.id);
    upload(&router, &db, post_owner, "cover.png", "image/png", PAYLOAD).await;
    upload(&router, &db, user_owner, "avatar.png", "image/png", PAYLOAD).await;

    let mut db_q = db.clone();
    let on_post = media_for_owner(&mut db_q, post_owner).await.unwrap();
    let on_user = media_for_owner(&mut db_q, user_owner).await.unwrap();
    assert_eq!(on_post.len(), 1, "the post's media");
    assert_eq!(on_user.len(), 1, "the user's media");
    assert_eq!(on_post[0].owner_type, "post");
    assert_eq!(on_user[0].owner_type, "user");
}

#[tokio::test]
async fn the_stored_row_renders_a_thumbnail_for_an_image_and_a_link_for_anything_else() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);
    upload(&router, &db, owner, "cover.png", "image/png", PAYLOAD).await;
    upload(&router, &db, owner, "notes.txt", "text/plain", "NOTES").await;

    let mut db_q = db.clone();
    let rows = media_for_owner(&mut db_q, owner).await.unwrap();
    let image = rows
        .iter()
        .find(|row| row.kind == KIND_IMAGE)
        .expect("the image row");
    let file = rows
        .iter()
        .find(|row| row.kind == KIND_FILE)
        .expect("the file row");

    let client = demo_client(&router, &db).await;
    let html = body_string(client.get(MEDIA_PATH).await).await;

    let thumbnail = tag_with(&html, &format!("src=\"{}\"", image.path));
    assert!(
        thumbnail.starts_with("<img"),
        "an image row must render a thumbnail, got {thumbnail}"
    );
    assert!(
        thumbnail.contains(&format!("alt=\"{}\"", image.filename)),
        "the thumbnail must name the file, got {thumbnail}"
    );
    let link = tag_with(&html, &format!("href=\"{}\"", file.path));
    assert!(
        link.starts_with("<a"),
        "anything that is not an image renders a link, got {link}"
    );
    assert!(
        !html.contains(&format!("href=\"{}\"", image.path)),
        "the image row is the thumbnail, not a link too: {html}"
    );
}

#[tokio::test]
async fn the_clear_control_clears_the_input_without_javascript() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let client = demo_client(&router, &db).await;
    let html = body_string(client.get(MEDIA_PATH).await).await;
    let form = upload_form(&html);

    let clear = tag_with(form, "data-media-clear");
    assert!(clear.starts_with("<button"), "got {clear}");
    assert!(
        clear.contains("type=\"reset\""),
        "the clear control must be a reset button: that is what empties the file \
         input with JavaScript off, got {clear}"
    );

    // The hooks the widget script consumes, and the region it draws into:
    // rendered hidden, because with no script there is no preview to show.
    for hook in ["data-media-file", "data-media-preview", "data-media-clear"] {
        assert!(form.contains(hook), "the form must render {hook}: {form}");
    }
    let preview = tag_with(form, "data-media-preview");
    assert!(
        preview.contains("hidden"),
        "the preview region starts hidden, got {preview}"
    );

    // The clear control belongs to the form whose file input it clears.
    assert!(
        form.contains("name=\"file\""),
        "the file input is the form's own: {form}"
    );
}

#[tokio::test]
async fn an_upload_for_an_owner_that_does_not_exist_is_refused() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let before = media_count(&db).await;

    let response = upload(
        &router,
        &db,
        MediaOwner::Post(uuid::Uuid::new_v4()),
        "cover.png",
        "image/png",
        PAYLOAD,
    )
    .await;

    assert_eq!(
        response.status(),
        400,
        "a polymorphic pair carries no foreign key, so the app refuses a dangling owner"
    );
    assert_eq!(media_count(&db).await, before, "and writes no row");
}

#[tokio::test]
async fn an_upload_without_the_csrf_token_is_refused() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let before = media_count(&db).await;
    let post = published_post(&db).await;
    let client = demo_client(&router, &db).await;

    let response = post_upload(
        &client,
        MediaOwner::Post(post.id),
        "cover.png",
        "image/png",
        PAYLOAD,
        None,
    )
    .await;

    assert_eq!(response.status(), 403, "the app's own form is CSRF-checked");
    assert_eq!(media_count(&db).await, before, "and writes no row");
}

#[tokio::test]
async fn a_tenantless_request_is_refused() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let client = tenantless_client(&router, &db).await;

    assert_eq!(
        client.get(MEDIA_PATH).await.status(),
        403,
        "the library lists one tenant's media, so a tenantless request is refused"
    );

    let before = media_count(&db).await;
    let response = post_upload(
        &client,
        MediaOwner::Post(post.id),
        "cover.png",
        "image/png",
        PAYLOAD,
        Some(&uuid::Uuid::new_v4().to_string()),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(media_count(&db).await, before, "and writes no row");
}

#[tokio::test]
async fn a_client_filename_is_stored_as_a_basename_inside_the_served_directory() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);

    upload(
        &router,
        &db,
        owner,
        "../../escape.png",
        "image/png",
        PAYLOAD,
    )
    .await;

    let mut db_q = db.clone();
    let row = media_for_owner(&mut db_q, owner).await.unwrap().remove(0);
    assert_eq!(row.filename, "escape.png", "the row keeps the basename");
    assert!(
        !row.path.contains(".."),
        "the stored URL cannot climb out of the served directory, got {}",
        row.path
    );

    // Serving it back is what proves it landed inside the directory the panel
    // serves: a file written anywhere else is not reachable at this path.
    let client = demo_client(&router, &db).await;
    assert_eq!(
        client.get(&row.path).await.status(),
        200,
        "{} must be inside the served directory",
        row.path
    );
}

#[tokio::test]
async fn a_filename_that_would_break_the_url_still_fetches_back() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);
    let client = demo_client(&router, &db).await;

    // Every name here reaches the store as the browser sent it, and every one
    // must come back: `#` would start a fragment, a space would end the URL,
    // `%22` is what Chrome sends for a quote, and a `%` would decode to
    // something the file on disk is not named.
    for (sent, recorded) in [
        ("cover #1.png", "cover #1.png"),
        (
            "quote%22 onerror=%22boom.png",
            "quote%22 onerror=%22boom.png",
        ),
        ("100%.png", "100%.png"),
        ("trailing .png ", "trailing .png"),
    ] {
        let csrf = uuid::Uuid::new_v4().to_string();
        let response = post_upload(&client, owner, sent, "image/png", PAYLOAD, Some(&csrf)).await;
        assert!(
            response.status().is_redirection(),
            "{sent:?} must save, got {}",
            response.status()
        );

        let mut db_q = db.clone();
        let rows = media_for_owner(&mut db_q, owner).await.unwrap();
        let row = rows
            .iter()
            .find(|row| row.filename == recorded)
            .unwrap_or_else(|| panic!("no row recorded {recorded:?} for {sent:?}"));
        assert!(
            !row.path.contains(['#', '"', ' ']),
            "{sent:?} stored a path that is not one URL segment: {}",
            row.path
        );

        let response = client.get(&row.path).await;
        assert_eq!(
            response.status(),
            200,
            "{} must resolve to the stored bytes",
            row.path
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect the served file")
            .to_bytes();
        assert_eq!(
            bytes.as_ref(),
            PAYLOAD.as_bytes(),
            "{} served the wrong bytes",
            row.path
        );
    }
}

#[tokio::test]
async fn the_blog_post_page_shows_the_media_attached_to_the_post() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);
    upload(&router, &db, owner, "cover.png", "image/png", PAYLOAD).await;

    let mut db_q = db.clone();
    let row = media_for_owner(&mut db_q, owner).await.unwrap().remove(0);

    // Anonymous, like any reader of the public blog.
    let html = body_string(
        TestClient::new(&router)
            .get(&format!("/blog/{}", post.id))
            .await,
    )
    .await;
    let thumbnail = tag_with(&html, &format!("src=\"{}\"", row.path));
    assert!(
        thumbnail.starts_with("<img"),
        "the post's page must show its media, got {thumbnail}"
    );
}

#[tokio::test]
async fn the_library_lists_one_tenants_rows() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let mut db_q = db.clone();
    let user = User::all().exec(&mut db_q).await.unwrap().remove(0);
    let owner = MediaOwner::User(user.id);

    // A user is global in this app, so another tenant can attach media to one:
    // the row carries the tenant that uploaded it.
    let other = uuid::Uuid::from_u128(4242);
    let csrf = uuid::Uuid::new_v4().to_string();
    let response = post_upload(
        &demo_client(&router, &db).await.tenant(other),
        owner,
        "avatar.png",
        "image/png",
        PAYLOAD,
        Some(&csrf),
    )
    .await;
    assert!(
        response.status().is_redirection(),
        "the upload must save for its own tenant, got {}",
        response.status()
    );

    let html = body_string(demo_client(&router, &db).await.get(MEDIA_PATH).await).await;
    assert!(
        html.contains("No media has been uploaded yet."),
        "another tenant's media must not be listed: {html}"
    );
}

#[tokio::test]
async fn a_row_whose_owner_is_gone_still_renders() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let post = published_post(&db).await;
    let owner = MediaOwner::Post(post.id);
    upload(&router, &db, owner, "cover.png", "image/png", PAYLOAD).await;

    // The pair carries no foreign key, so nothing cascades: the row stays, and
    // the page says what happened to its owner (ADR-0021).
    let mut db_q = db.clone();
    Post::filter(Post::fields().id().eq(post.id))
        .delete()
        .exec(&mut db_q)
        .await
        .expect("delete the post the media was uploaded for");

    let html = body_string(demo_client(&router, &db).await.get(MEDIA_PATH).await).await;
    assert!(
        html.contains("Post · (deleted)"),
        "the row must still render, named as dangling: {html}"
    );
}
