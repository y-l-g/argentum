//! The media library: the app-level upload path writes a `medias` row,
//! the stored rows render a thumbnail or a link, and the clear control works
//! with and without JavaScript.
//!
//! The widget's JavaScript half is `examples/showcase/assets/media.test.js`
//! (Node); what a server-rendered page can pin is the markup contract those
//! hooks describe — the clear control is a reset button, so a browser empties
//! the file input with no script at all.

use http_body_util::BodyExt;
use showcase::{
    app::router_with_app_uploads,
    media::{KIND_FILE, KIND_IMAGE, MEDIA_PATH},
    models::{DEMO_TENANT, MediaAsset},
};
use topcoat::router::{Body, Router};

use crate::common::{TestClient, body_string, demo_client, full_db, tenantless_client};

/// Bytes of an uploaded file: ASCII, so the multipart body can be a `String`,
/// which is all the test client takes.
const PAYLOAD: &str = "PNG-FAKE-BYTES";

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
        "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n{payload}\r\n\
         {token}--{b}--\r\n",
        b = boundary,
    );
    client.post_multipart(MEDIA_PATH, boundary, body).await
}

/// One upload with a freshly minted CSRF pair, as the demo admin.
async fn upload(
    router: &Router,
    db: &toasty::Db,
    filename: &str,
    content_type: &str,
    payload: &str,
) -> http::Response<Body> {
    let client = demo_client(router, db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    post_upload(&client, filename, content_type, payload, Some(&csrf)).await
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
async fn an_upload_creates_a_row() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());

    let response = upload(&router, &db, "cover.png", "image/png", PAYLOAD).await;
    assert!(
        response.status().is_redirection(),
        "the upload must save, got {}",
        response.status()
    );

    let mut db_q = db.clone();
    let rows = MediaAsset::all().exec(&mut db_q).await.unwrap();
    assert_eq!(rows.len(), 1, "one row for the upload");
    let row = &rows[0];
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
async fn the_upload_form_is_file_only() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let client = demo_client(&router, &db).await;
    let html = body_string(client.get(MEDIA_PATH).await).await;
    let form = upload_form(&html);
    assert!(
        form.contains("name=\"file\""),
        "the form must offer a file input: {form}"
    );
    assert!(
        !form.contains("name=\"owner\""),
        "the form must not offer an owner picker: {form}"
    );
    assert!(
        !form.contains("<select"),
        "a file-only form renders no select: {form}"
    );
}

#[tokio::test]
async fn the_stored_row_renders_a_thumbnail_for_an_image_and_a_link_for_anything_else() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    upload(&router, &db, "cover.png", "image/png", PAYLOAD).await;
    upload(&router, &db, "notes.txt", "text/plain", "NOTES").await;

    let mut db_q = db.clone();
    let rows = MediaAsset::all().exec(&mut db_q).await.unwrap();
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
async fn an_upload_without_the_csrf_token_is_refused() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let before = media_count(&db).await;
    let client = demo_client(&router, &db).await;

    let response = post_upload(&client, "cover.png", "image/png", PAYLOAD, None).await;

    assert_eq!(response.status(), 403, "the app's own form is CSRF-checked");
    assert_eq!(media_count(&db).await, before, "and writes no row");
}

#[tokio::test]
async fn a_tenantless_request_is_refused() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    let client = tenantless_client(&router, &db).await;

    assert_eq!(
        client.get(MEDIA_PATH).await.status(),
        403,
        "the library lists one tenant's media, so a tenantless request is refused"
    );

    let before = media_count(&db).await;
    let response = post_upload(
        &client,
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

    upload(&router, &db, "../../escape.png", "image/png", PAYLOAD).await;

    let mut db_q = db.clone();
    let row = MediaAsset::all().exec(&mut db_q).await.unwrap().remove(0);
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
        let response = post_upload(&client, sent, "image/png", PAYLOAD, Some(&csrf)).await;
        assert!(
            response.status().is_redirection(),
            "{sent:?} must save, got {}",
            response.status()
        );

        let mut db_q = db.clone();
        let rows = MediaAsset::all().exec(&mut db_q).await.unwrap();
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
async fn a_picked_cover_renders_on_the_blog_post_page() {
    use showcase::models::{Author, DEMO_TENANT, Post, Publication, Seo};

    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());
    upload(&router, &db, "cover.png", "image/png", PAYLOAD).await;

    let mut db_q = db.clone();
    let row = MediaAsset::all().exec(&mut db_q).await.unwrap().remove(0);
    let author = Author::all()
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("a seeded author");
    let post = toasty::create!(Post {
        id: uuid::Uuid::new_v4(),
        tenant_id: DEMO_TENANT,
        title: "Cover Post",
        body: "Body with a cover.",
        status: "published".to_string(),
        featured: false,
        created_at: "2024-03-01T09:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        cover_id: Some(row.id),
        tags: String::new(),
        seo: Seo {
            title: String::new(),
            description: String::new(),
        },
        publication: Publication::Published {
            published_at: "2024-03-01T09:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            canonical_url: String::new(),
        },
        author_id: author.id,
    })
    .exec(&mut db_q)
    .await
    .expect("create the covered post");

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
        "the post's page must show its picked cover, got {thumbnail}"
    );
}

#[tokio::test]
async fn the_library_lists_one_tenants_rows() {
    let db = full_db().await;
    let router = router_with_app_uploads(db.clone());

    // Another tenant uploads: the row carries the tenant that uploaded it.
    let other = uuid::Uuid::from_u128(4242);
    let csrf = uuid::Uuid::new_v4().to_string();
    let response = post_upload(
        &demo_client(&router, &db).await.tenant(other),
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
