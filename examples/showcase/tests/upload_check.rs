//! The upload demo end to end: the showcase's own uploader writes a
//! multipart part into the directory the panel serves, the record stores the
//! URL it returned, and that URL fetches the bytes back.
//!
//! This is the half no framework test can cover: `serve_dir` and the store are
//! both the app's, and the contract between them is only the path string.

use std::path::PathBuf;

use http_body_util::BodyExt;
use showcase::{
    app::{UPLOAD_URL_PREFIX, router_with_uploads},
    models::{Author, Post},
};

use crate::common::{body_string, demo_client, full_db, input_value, multipart_body};

/// A directory of this test's own — the demo's own upload dir is shared state.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("showcase-upload-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The bytes of the uploaded file: ASCII so the multipart body can be a
/// `String`, which is all the test client takes.
const PAYLOAD: &str = "PNG-FAKE-BYTES";

/// Create a post through the demo uploader, returning the path the record
/// stores.
async fn upload_a_cover_image(
    router: &topcoat::router::Router,
    db: &toasty::Db,
) -> (String, uuid::Uuid) {
    let client = demo_client(router, db).await;
    let mut db_q = db.clone();
    let author = Author::all().exec(&mut db_q).await.unwrap().remove(0);
    let csrf = uuid::Uuid::new_v4().to_string();
    let boundary = "----UploadBoundary";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nUploaded Cover\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"author_id\"\r\n\r\n{id}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"cover.png\"\r\nContent-Type: image/png\r\n\r\n{payload}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nupload\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{csrf}\r\n\
         --{b}--\r\n",
        b = boundary,
        id = author.id,
        payload = PAYLOAD,
    );
    let response = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
        .await;
    assert!(
        response.status().is_redirection(),
        "the upload must save, got {}",
        response.status()
    );

    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().title().eq("Uploaded Cover".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the uploaded post");
    (post.image_path, post.id)
}

#[tokio::test]
async fn an_uploaded_file_lands_in_the_served_directory_and_fetches_back() {
    let db = full_db().await;
    let dir = temp_dir("fetch");
    let router = router_with_uploads(db.clone(), dir.clone());

    let (stored, _) = upload_a_cover_image(&router, &db).await;

    // The record holds the store's URL, not the client's filename.
    assert!(
        stored.starts_with(&format!("{UPLOAD_URL_PREFIX}/")),
        "the stored path must be the served URL, got {stored}"
    );
    assert!(stored.ends_with("cover.png"), "got {stored}");

    // The bytes are on disk in the directory the app configured...
    let name = stored
        .strip_prefix(&format!("{UPLOAD_URL_PREFIX}/"))
        .expect("stored path carries the served prefix");
    let on_disk = std::fs::read(dir.join(name)).expect("the upload is written to the served dir");
    assert_eq!(on_disk, PAYLOAD.as_bytes());

    // ...and the stored path — exactly the string in the record — fetches them
    // back through the panel's own router.
    let client = demo_client(&router, &db).await;
    let response = client.get(&stored).await;
    assert_eq!(response.status(), 200, "{stored} must be fetchable");
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/png"),
        "the served file keeps its type"
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect the served file")
        .to_bytes();
    assert_eq!(bytes.as_ref(), PAYLOAD.as_bytes());
}

#[tokio::test]
async fn the_edit_page_links_the_stored_upload_and_offers_to_remove_it() {
    let db = full_db().await;
    let router = router_with_uploads(db.clone(), temp_dir("link"));
    let (stored, id) = upload_a_cover_image(&router, &db).await;

    let client = demo_client(&router, &db).await;
    let response = client.get(&format!("/admin/posts/{id}/edit")).await;
    assert!(response.status().is_success());
    let html = body_string(response).await;
    assert!(
        !html.contains(&format!("src=\"{stored}\"")),
        "the edit form renders no image preview of the upload: {html}"
    );
    assert!(
        html.contains(&format!("href=\"{stored}\"")),
        "the edit form must link the stored upload: {html}"
    );
    assert!(
        html.contains("name=\"clear_image_path\""),
        "the edit form must offer the clear control: {html}"
    );
}

/// A create that stored a file and then failed another field's validation
/// carries the upload into the re-rendered form: the browser's file
/// input is empty on the next attempt, so without the carry the create fails
/// `required` and the stored file is lost.
#[tokio::test]
async fn a_re_rendered_create_keeps_the_upload() {
    let db = full_db().await;
    let dir = temp_dir("create-rerender");
    let router = router_with_uploads(db.clone(), dir.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let author = Author::all().exec(&mut db_q).await.unwrap().remove(0);

    let csrf = uuid::Uuid::new_v4().to_string();
    let boundary = "----CarryCreateBoundary";
    let author_id = author.id.to_string();
    // The file is stored, but the required title is empty, so the form
    // re-renders with the error.
    let body = multipart_body(
        boundary,
        &[
            ("title", ""),
            ("author_id", &author_id),
            ("tags", "carry"),
            ("csrf_token", &csrf),
        ],
        &[("image_path", "cover.png", PAYLOAD)],
    );
    let response = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
        .await;
    assert_eq!(response.status(), 200, "the invalid create re-renders");
    let html = body_string(response).await;
    assert!(html.contains("Title is required"), "got {html}");
    let carried = input_value(&html, "keep_image_path")
        .expect("the re-rendered create must carry the stored upload");
    assert!(
        carried.starts_with(&format!("{UPLOAD_URL_PREFIX}/")),
        "the carried value is the store's answer, got {carried}"
    );
    assert!(
        html.contains(&format!("data-file-current=\"{carried}\"")),
        "the carried path is the one the form shows as current, got {html}"
    );

    // What the browser submits next: the form's fields and an empty file input.
    let body = multipart_body(
        boundary,
        &[
            ("title", "Carried Cover"),
            ("author_id", &author_id),
            ("tags", "carry"),
            ("keep_image_path", &carried),
            ("csrf_token", &csrf),
        ],
        &[("image_path", "", "")],
    );
    let response = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
        .await;
    assert!(
        response.status().is_redirection(),
        "the corrected create must save, got {} {}",
        response.status(),
        body_string(response).await
    );
    let post = Post::filter(Post::fields().title().eq("Carried Cover".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the created post");
    assert_eq!(
        post.image_path, carried,
        "the upload survives the re-render instead of failing required"
    );
    // And it is the bytes that were uploaded first, not a second write.
    let name = carried
        .strip_prefix(&format!("{UPLOAD_URL_PREFIX}/"))
        .expect("the stored path carries the served prefix");
    assert_eq!(
        std::fs::read(dir.join(name)).expect("the first upload is on disk"),
        PAYLOAD.as_bytes()
    );
}

/// An edit that uploaded a replacement and then failed another field's
/// validation keeps the replacement, not the record's old file.
#[tokio::test]
async fn a_re_rendered_edit_keeps_the_new_upload() {
    let db = full_db().await;
    let router = router_with_uploads(db.clone(), temp_dir("edit-rerender"));
    let (old, id) = upload_a_cover_image(&router, &db).await;
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().id().eq(id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the uploaded post");
    let author_id = post.author_id.to_string();

    let csrf = uuid::Uuid::new_v4().to_string();
    let boundary = "----CarryEditBoundary";
    let edit_body = |title: &str, keep: &str, file: &[(&str, &str, &str)]| {
        multipart_body(
            boundary,
            &[
                ("title", title),
                ("author_id", &author_id),
                ("tags", "carry"),
                ("body", "Body"),
                ("status", "published"),
                ("featured", "true"),
                ("seo_title", "S"),
                ("seo_description", "D"),
                ("media", "1"),
                ("media_url", "/i.jpg"),
                ("media_alt", "alt"),
                ("post_stats_word_count", "1"),
                ("post_stats_read_minutes", "1"),
                ("keep_image_path", keep),
                ("csrf_token", &csrf),
            ],
            file,
        )
    };
    let url = format!("/admin/posts/{id}/edit");

    // A replacement is uploaded, the title is empty, and the form re-renders.
    let response = client
        .csrf(&csrf)
        .post_multipart(
            &url,
            boundary,
            edit_body("", "", &[("image_path", "new.png", PAYLOAD)]),
        )
        .await;
    assert_eq!(response.status(), 200, "the invalid edit re-renders");
    let html = body_string(response).await;
    assert!(html.contains("Title is required"), "got {html}");
    let carried = input_value(&html, "keep_image_path")
        .expect("the re-rendered edit must carry the new upload");
    assert_ne!(
        carried, old,
        "the carry is the replacement, not the old file"
    );
    assert!(
        html.contains(&format!("data-file-current=\"{carried}\"")),
        "the form shows the replacement as current, got {html}"
    );

    // The next submit carries the replacement and an empty file input.
    let response = client
        .csrf(&csrf)
        .post_multipart(
            &url,
            boundary,
            edit_body("Edited With Cover", &carried, &[("image_path", "", "")]),
        )
        .await;
    assert!(
        response.status().is_redirection(),
        "the corrected edit must save, got {} {}",
        response.status(),
        body_string(response).await
    );
    let saved = Post::filter(Post::fields().id().eq(id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        saved.image_path, carried,
        "the record keeps the new upload, not the old file"
    );
}

/// The carry does not re-open GH #277: a client-typed
/// `keep_<field>` is used only when the installed store still holds the path,
/// so a forged one never reaches the record.
#[tokio::test]
async fn a_forged_carried_upload_is_refused() {
    let db = full_db().await;
    let router = router_with_uploads(db.clone(), temp_dir("forged-carry"));
    let (old, id) = upload_a_cover_image(&router, &db).await;
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().id().eq(id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the uploaded post");
    let author_id = post.author_id.to_string();

    let csrf = uuid::Uuid::new_v4().to_string();
    let boundary = "----ForgedCarryBoundary";
    const FORGED: &str = "javascript:alert(1)";
    // Create: nothing stored the forged path, so the required field stays empty
    // and the record is not written with it.
    let body = multipart_body(
        boundary,
        &[
            ("title", "Forged Carry"),
            ("author_id", &author_id),
            ("tags", "x"),
            ("keep_image_path", FORGED),
            ("csrf_token", &csrf),
        ],
        &[("image_path", "", "")],
    );
    let response = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
        .await;
    assert_eq!(response.status(), 200, "the forged create re-renders");
    let html = body_string(response).await;
    assert!(
        html.contains("Cover image is required"),
        "a forged carry leaves the field empty and required, got {html}"
    );
    assert!(
        Post::filter(Post::fields().title().eq("Forged Carry".to_string()))
            .exec(&mut db_q)
            .await
            .unwrap()
            .is_empty(),
        "a forged carry must not create a record"
    );

    // Edit: the forged carry is ignored and the record keeps its stored file.
    let body = multipart_body(
        boundary,
        &[
            ("title", "Forged Edit"),
            ("author_id", &author_id),
            ("tags", "x"),
            ("body", "Body"),
            ("keep_image_path", FORGED),
            ("csrf_token", &csrf),
        ],
        &[("image_path", "", "")],
    );
    let response = client
        .csrf(&csrf)
        .post_multipart(&format!("/admin/posts/{id}/edit"), boundary, body)
        .await;
    assert!(
        response.status().is_redirection(),
        "the forged edit saves the record unchanged, got {} {}",
        response.status(),
        body_string(response).await
    );
    let saved = Post::filter(Post::fields().id().eq(id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        saved.image_path, old,
        "a forged carry must not replace the stored file"
    );
}
