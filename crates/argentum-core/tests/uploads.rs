//! The upload seam end to end (GH #188): what an installed [`Uploader`] does
//! with a `FileUpload`'s bytes, what happens when it refuses, what the clear
//! control empties, and that `Panel::serve_dir` hands a stored path back.
//!
//! The pre-#188 contract is pinned here too: with no uploader installed the
//! sanitized basename is still the stored value, so installing the seam is
//! additive for every app that never installs one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use argentum_core::{
    Auth, FileUpload, Panel, Resource, Schema, Table, TextColumn, TextInput, Uploader,
};
use http::header::{CONTENT_TYPE, COOKIE};
use toasty::Db;
use topcoat::context::Cx;
use topcoat::router::response::Response;
use topcoat::router::{Body, Router};
use uuid::Uuid;

/// A document with one required and one optional upload: the two ends of the
/// clear-control rule (GH #188).
#[derive(Debug, Clone, toasty::Model)]
struct Doc {
    #[key]
    #[auto]
    id: Uuid,
    title: String,
    /// Required by the form's default (the lens is a non-nullable `String`).
    cover: String,
    /// Declared `.optional()`: the app allows a record to lose its file.
    attachment: String,
}

/// What an uploader was handed: sanitized filename and bytes.
type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// An uploader that records what it was handed and answers a deterministic
/// path, so a test can prove the *bytes* arrived and not just the name.
#[derive(Clone, Default)]
struct RecordingUploader {
    seen: Seen,
}

impl RecordingUploader {
    fn seen(&self) -> Vec<(String, Vec<u8>)> {
        self.seen.lock().expect("uploader lock").clone()
    }
}

impl Uploader for RecordingUploader {
    async fn store(&self, filename: &str, bytes: &[u8]) -> Result<String, String> {
        self.seen
            .lock()
            .expect("uploader lock")
            .push((filename.to_string(), bytes.to_vec()));
        Ok(format!("/uploads/{filename}"))
    }
}

/// An uploader that always refuses, for the inline-error path.
struct FailingUploader;

impl Uploader for FailingUploader {
    async fn store(&self, _filename: &str, _bytes: &[u8]) -> Result<String, String> {
        Err("this deployment has no room left".to_string())
    }
}

struct DocResource;

impl Resource for DocResource {
    type Model = Doc;

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    // Every policy hook defaults to deny, so a panel that only exercises the
    // upload seam opens them all (the permissive end of the contract).
    fn can_view(_cx: &Cx, _record: &Doc) -> bool {
        true
    }

    fn can_create(_cx: &Cx) -> bool {
        true
    }

    fn can_update(_cx: &Cx, _record: &Doc) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Doc> {
        Table::r#for(cx)
            .id(|doc: &Doc| doc.id.to_string())
            .pk(|doc: &Doc| doc.id.to_string())
            .paginate(25)
            .columns(TextColumn::r#for(Doc::fields().title(), |doc: &Doc| {
                doc.title.clone()
            }))
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            TextInput::r#for(Doc::fields().title()),
            FileUpload::r#for(Doc::fields().cover()).label("Cover"),
            FileUpload::r#for(Doc::fields().attachment())
                .label("Attachment")
                .optional(),
        ))
    }

    fn hydrate_form_values(record: &Doc) -> HashMap<String, String> {
        HashMap::from([
            ("title".to_string(), record.title.clone()),
            ("cover".to_string(), record.cover.clone()),
            ("attachment".to_string(), record.attachment.clone()),
        ])
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Doc> {
        // Absent keys store "" (GH #89) — the shape every showcase record fn
        // has, so the upload path reaches the row the ordinary way.
        let (title, cover, attachment) = stored_values(&values);
        toasty::create!(Doc {
            title: title,
            cover: cover,
            attachment: attachment,
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })
    }

    async fn update_record(
        _cx: &Cx,
        mut record: Doc,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Doc> {
        // Absent keys keep the stored value (GH #89); a cleared upload arrives
        // as a present, empty value.
        for (name, value) in [
            ("title", &mut record.title),
            ("cover", &mut record.cover),
            ("attachment", &mut record.attachment),
        ] {
            if let Some(submitted) = values.get(name) {
                *value = submitted.trim().to_string();
            }
        }
        toasty::update!(record {
            title: record.title.clone(),
            cover: record.cover.clone(),
            attachment: record.attachment.clone(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })?;
        Ok(record)
    }
}

/// The three uploaded strings a record fn reads, trimmed like every app does.
fn stored_values(values: &HashMap<String, String>) -> (String, String, String) {
    let read = |name: &str| {
        values
            .get(name)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    (read("title"), read("cover"), read("attachment"))
}

async fn seeded_db() -> Db {
    let db = Db::builder()
        .models(toasty::models!(Doc))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    db
}

/// A panel over `Doc`, optionally with an uploader — the seam's own on/off
/// switch, which is the whole point of the default being additive.
fn router(db: Db, uploader: Option<impl Uploader>) -> Router {
    let panel = Panel::new("admin")
        .app_context(db)
        // The upload seam is what these tests exercise; the auth gate is
        // covered by its own suite.
        .auth(Auth::disabled());
    let panel = match uploader {
        Some(uploader) => panel.uploads(uploader),
        None => panel,
    };
    panel
        .resource::<DocResource>()
        .build()
        .expect("panel builds")
}

/// A directory of this test's own, removed with the process's temp dir.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("argentum-uploads-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A multipart body, one part per entry: `None` is a text part, `Some("")` the
/// browser's "no file chosen" file part, `Some(name)` a chosen file.
fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &str)]) -> String {
    let mut body = String::new();
    for (name, filename, content) in parts {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""
        ));
        if let Some(filename) = filename {
            body.push_str(&format!("; filename=\"{filename}\""));
        }
        body.push_str("\r\n\r\n");
        body.push_str(content);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    body
}

/// A POST carrying a matching CSRF cookie + field (the double-submit pair).
async fn post(
    router: &Router,
    uri: &str,
    csrf: &str,
    content_type: String,
    body: String,
) -> Response<Body> {
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(CONTENT_TYPE, content_type)
        .header(
            COOKIE,
            format!("{}={csrf}", argentum_core::csrf::COOKIE_NAME),
        )
        .body(Body::from(body))
        .expect("request builds");
    router.handle(request).await
}

/// POST a multipart form (every upload form's enctype).
async fn post_multipart(
    router: &Router,
    uri: &str,
    csrf: &str,
    boundary: &str,
    body: String,
) -> Response<Body> {
    post(
        router,
        uri,
        csrf,
        format!("multipart/form-data; boundary={boundary}"),
        body,
    )
    .await
}

async fn get(router: &Router, uri: &str) -> Response<Body> {
    let request = http::Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    router.handle(request).await
}

async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec()
}

async fn body_string(response: Response<Body>) -> String {
    String::from_utf8_lossy(&body_bytes(response).await).into_owned()
}

/// A new CSRF token, paired with the cookie `post` sends.
fn new_csrf() -> String {
    Uuid::new_v4().to_string()
}

async fn seed_doc(db: &Db, title: &str, cover: &str, attachment: &str) -> Doc {
    let mut db = db.clone();
    toasty::create!(Doc {
        title: title.to_string(),
        cover: cover.to_string(),
        attachment: attachment.to_string(),
    })
    .exec(&mut db)
    .await
    .expect("seed doc")
}

async fn docs(db: &Db) -> Vec<Doc> {
    let mut db = db.clone();
    Doc::all().exec(&mut db).await.expect("query docs")
}

#[tokio::test]
async fn an_installed_uploader_stores_the_bytes_and_the_path_reaches_the_record() {
    let db = seeded_db().await;
    let uploader = RecordingUploader::default();
    let router = router(db.clone(), Some(uploader.clone()));
    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Notes"),
            ("cover", Some("cover.png"), "PNG-BYTES"),
            ("attachment", Some("spec.pdf"), "PDF-BYTES"),
            ("csrf_token", None, &csrf),
        ],
    );

    let response = post_multipart(&router, "/admin/docs/create", &csrf, "B", body).await;
    assert_eq!(response.status(), 303, "a valid create redirects");

    // The record stores what the uploader returned, not the client filename.
    let created = docs(&db).await;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].cover, "/uploads/cover.png");
    assert_eq!(created[0].attachment, "/uploads/spec.pdf");

    // And the uploader saw the bytes under sanitized names. Order is not part
    // of the contract (uploads are independent), so the pairs are sorted.
    let mut seen = uploader.seen();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("cover.png".to_string(), b"PNG-BYTES".to_vec()),
            ("spec.pdf".to_string(), b"PDF-BYTES".to_vec()),
        ],
        "the uploader receives each file part's sanitized name and content"
    );
}

#[tokio::test]
async fn without_an_uploader_the_sanitized_basename_is_still_stored() {
    // The pre-#188 contract, and the reason the seam is additive: an app that
    // installs nothing keeps exactly what it had.
    let db = seeded_db().await;
    let router = router(db.clone(), None::<RecordingUploader>);
    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Notes"),
            // A path-carrying client name is sanitized to its basename (GH #90).
            ("cover", Some("../../etc/cover.png"), "PNG-BYTES"),
            ("csrf_token", None, &csrf),
        ],
    );

    let response = post_multipart(&router, "/admin/docs/create", &csrf, "B", body).await;
    assert_eq!(response.status(), 303);

    let created = docs(&db).await;
    assert_eq!(created[0].cover, "cover.png");
    assert_eq!(created[0].attachment, "");
}

#[tokio::test]
async fn a_refused_upload_is_an_inline_field_error_and_writes_nothing() {
    let db = seeded_db().await;
    let router = router(db.clone(), Some(FailingUploader));
    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Notes"),
            ("cover", Some("cover.png"), "PNG-BYTES"),
            ("csrf_token", None, &csrf),
        ],
    );

    let response = post_multipart(&router, "/admin/docs/create", &csrf, "B", body).await;
    // A rejected upload is user input, not infrastructure: the form comes back
    // with the reason against the field, and no 500 page.
    assert_eq!(response.status(), 200, "the form re-renders");
    let html = body_string(response).await;
    assert!(
        html.contains("Cover could not be uploaded: this deployment has no room left"),
        "the uploader's reason must reach the field's inline error: {html}"
    );
    assert!(
        !html.contains("Cover is required"),
        "'required' would restate the symptom and hide the reason: {html}"
    );
    assert!(
        !html.contains("data-file-current"),
        "a create must not present the refused filename as a stored file: {html}"
    );
    assert!(
        docs(&db).await.is_empty(),
        "a refused upload must not create the record"
    );
}

#[tokio::test]
async fn an_untouched_file_input_keeps_the_stored_path_and_a_chosen_one_replaces_it() {
    let db = seeded_db().await;
    let uploader = RecordingUploader::default();
    let router = router(db.clone(), Some(uploader.clone()));
    let doc = seed_doc(&db, "Original", "cover.png", "spec.pdf").await;

    // A browser submits every file input; untouched ones arrive with an empty
    // filename (GH #90/GH #184).
    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Renamed"),
            ("cover", Some(""), ""),
            ("attachment", Some(""), ""),
            ("csrf_token", None, &csrf),
        ],
    );
    let response = post_multipart(
        &router,
        &format!("/admin/docs/{}/edit", doc.id),
        &csrf,
        "B",
        body,
    )
    .await;
    assert_eq!(response.status(), 303, "an untouched upload saves");
    let updated = docs(&db).await;
    assert_eq!(updated[0].title, "Renamed");
    assert_eq!(updated[0].cover, "cover.png", "the stored path is kept");
    assert_eq!(updated[0].attachment, "spec.pdf");
    assert!(
        uploader.seen().is_empty(),
        "an untouched file input must not reach the uploader"
    );

    // Choosing a file replaces the stored one: the new path is what is stored.
    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Renamed"),
            ("cover", Some("new.png"), "NEW-BYTES"),
            ("attachment", Some(""), ""),
            ("csrf_token", None, &csrf),
        ],
    );
    let response = post_multipart(
        &router,
        &format!("/admin/docs/{}/edit", doc.id),
        &csrf,
        "B",
        body,
    )
    .await;
    assert_eq!(response.status(), 303);
    assert_eq!(docs(&db).await[0].cover, "/uploads/new.png");
}

#[tokio::test]
async fn clearing_an_optional_upload_empties_the_stored_path() {
    let db = seeded_db().await;
    let router = router(db.clone(), Some(RecordingUploader::default()));
    let doc = seed_doc(&db, "Original", "cover.png", "spec.pdf").await;

    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Original"),
            ("cover", Some(""), ""),
            ("attachment", Some(""), ""),
            // The framework's own control posts this (GH #188).
            ("clear_attachment", None, "1"),
            ("csrf_token", None, &csrf),
        ],
    );
    let response = post_multipart(
        &router,
        &format!("/admin/docs/{}/edit", doc.id),
        &csrf,
        "B",
        body,
    )
    .await;
    assert_eq!(response.status(), 303, "clearing an optional upload saves");

    let updated = docs(&db).await;
    assert_eq!(updated[0].attachment, "", "the cleared field is emptied");
    assert_eq!(updated[0].cover, "cover.png", "the untouched one is kept");
}

#[tokio::test]
async fn clearing_a_required_upload_is_refused_inline() {
    // `required` is not waived by an explicit clear: a record that must have a
    // file cannot lose it, and the refusal is the ordinary required error
    // rather than a silent empty write (GH #188).
    let db = seeded_db().await;
    let router = router(db.clone(), Some(RecordingUploader::default()));
    let doc = seed_doc(&db, "Original", "cover.png", "spec.pdf").await;

    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Original"),
            ("cover", Some(""), ""),
            ("attachment", Some(""), ""),
            ("clear_cover", None, "1"),
            ("csrf_token", None, &csrf),
        ],
    );
    let response = post_multipart(
        &router,
        &format!("/admin/docs/{}/edit", doc.id),
        &csrf,
        "B",
        body,
    )
    .await;
    assert_eq!(response.status(), 200, "the form re-renders with the error");
    let html = body_string(response).await;
    assert!(
        html.contains("Cover is required"),
        "a required upload refuses the clear inline: {html}"
    );
    assert_eq!(
        docs(&db).await[0].cover,
        "cover.png",
        "the refused clear writes nothing"
    );
}

#[tokio::test]
async fn a_refused_edit_upload_keeps_showing_the_stored_file() {
    // The store refused, so nothing changed: the re-rendered form must still
    // show what is stored rather than the empty value a cleared field would
    // have (GH #188).
    let db = seeded_db().await;
    let router = router(db.clone(), Some(FailingUploader));
    let doc = seed_doc(&db, "Notes", "/uploads/old.png", "spec.pdf").await;

    let csrf = new_csrf();
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Renamed"),
            ("cover", Some("new.png"), "NEW-BYTES"),
            ("attachment", Some(""), ""),
            ("csrf_token", None, &csrf),
        ],
    );
    let response = post_multipart(
        &router,
        &format!("/admin/docs/{}/edit", doc.id),
        &csrf,
        "B",
        body,
    )
    .await;
    assert_eq!(response.status(), 200, "the form re-renders");
    let html = body_string(response).await;
    assert!(
        html.contains("Cover could not be uploaded: this deployment has no room left"),
        "the reason must reach the field: {html}"
    );
    assert!(
        html.contains("data-file-current=\"/uploads/old.png\""),
        "the stored file is still there and must still be shown: {html}"
    );
    assert_eq!(
        docs(&db).await[0].cover,
        "/uploads/old.png",
        "a refused upload writes nothing"
    );
}

#[tokio::test]
async fn an_over_cap_body_still_413s_with_an_uploader_installed() {
    // GH #188 item 5: the seam must not widen the body contract. The bytes are
    // buffered rather than drained in this configuration, so the cap is
    // asserted on the path that holds them.
    let db = seeded_db().await;
    let router = router(db.clone(), Some(RecordingUploader::default()));
    let csrf = new_csrf();
    let huge = "a".repeat(11 * 1024 * 1024);
    let body = multipart_body(
        "B",
        &[
            ("title", None, "Too big"),
            ("cover", Some("huge.png"), &huge),
            ("csrf_token", None, &csrf),
        ],
    );

    let response = post_multipart(&router, "/admin/docs/create", &csrf, "B", body).await;
    assert_eq!(
        response.status(),
        413,
        "an over-cap upload must 413 whether or not the bytes are buffered"
    );
    assert!(
        docs(&db).await.is_empty(),
        "an over-cap body must not create the record"
    );
}

#[tokio::test]
async fn the_edit_form_previews_an_image_and_links_any_other_file() {
    let db = seeded_db().await;
    let router = router(db.clone(), Some(RecordingUploader::default()));
    let doc = seed_doc(&db, "Notes", "/uploads/photo.png", "/files/spec.pdf").await;

    let response = get(&router, &format!("/admin/docs/{}/edit", doc.id)).await;
    assert!(response.status().is_success());
    let html = body_string(response).await;
    assert!(
        html.contains("<img") && html.contains("src=\"/uploads/photo.png\""),
        "an image path is previewed as an image: {html}"
    );
    assert!(
        !html.contains("href=\"/uploads/photo.png\""),
        "and a previewed image is not also a link: {html}"
    );
    assert!(
        html.contains("href=\"/files/spec.pdf\""),
        "a non-image path is a link to the file: {html}"
    );
    assert!(
        !html.contains("src=\"/files/spec.pdf\""),
        "and a non-image is not rendered as an image: {html}"
    );
    // Both stored values offer the clear control, labelled with what it does.
    assert!(html.contains("name=\"clear_cover\""), "{html}");
    assert!(html.contains("name=\"clear_attachment\""), "{html}");
    assert!(html.contains("Remove the current file"), "{html}");
}

#[tokio::test]
async fn a_create_form_offers_neither_a_preview_nor_a_clear_control() {
    // Both belong to a stored value: a create has none, and an empty file
    // input cannot express "remove what is not there".
    let db = seeded_db().await;
    let router = router(db.clone(), Some(RecordingUploader::default()));

    let response = get(&router, "/admin/docs/create").await;
    assert!(response.status().is_success());
    let html = body_string(response).await;
    assert!(!html.contains("<img"), "{html}");
    assert!(!html.contains("name=\"clear_"), "{html}");
    assert!(
        html.contains("enctype=\"multipart/form-data\""),
        "a form with a file input posts multipart: {html}"
    );
}

#[tokio::test]
async fn serve_dir_serves_the_upload_directory_through_the_panel() {
    let db = seeded_db().await;
    let dir = temp_dir("serve");
    std::fs::write(dir.join("cat.png"), b"PNG-FILE").expect("write upload");

    let router = Panel::new("admin")
        .app_context(db)
        .auth(Auth::disabled())
        .serve_dir("/uploads/{*file}", dir.clone())
        .resource::<DocResource>()
        .build()
        .expect("panel builds");

    let response = get(&router, "/uploads/cat.png").await;
    assert_eq!(response.status(), 200, "the stored path is fetchable");
    assert_eq!(body_bytes(response).await, b"PNG-FILE");

    // The directory route's own rules hold through the panel, so exposing one
    // cannot widen them: a traversal attempt is not a file.
    let escaped = get(&router, "/uploads/%2e%2e/Cargo.toml").await;
    assert_eq!(
        escaped.status(),
        404,
        "a path out of the served directory is not served"
    );
    let missing = get(&router, "/uploads/absent.png").await;
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn a_serve_dir_path_without_a_catch_all_fails_the_build() {
    // `DirectoryRoute::new` panics on a pattern it cannot resolve; the panel
    // reports instead of panicking (GH #174), which is what `Panel::build`
    // returns a `Result` for.
    let db = seeded_db().await;
    let Err(error) = Panel::new("admin")
        .app_context(db)
        .serve_dir("/uploads", temp_dir("bad-path"))
        .resource::<DocResource>()
        .build()
    else {
        panic!("a serve_dir pattern with no catch-all must fail the build");
    };
    assert!(
        error.to_string().contains("catch-all"),
        "the error must name the fix: {error}"
    );
}
