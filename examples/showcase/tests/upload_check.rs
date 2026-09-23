//! The upload demo end to end (GH #188): the showcase's own uploader writes a
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

use crate::common::{body_string, demo_client, full_db};

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
