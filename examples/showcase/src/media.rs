//! The media library: the `medias` table and the page that fills it.
//!
//! A WordPress-style library: one row per stored file with the tenant that
//! uploaded it, the `path` the `Uploader` returned, the client's `filename`,
//! a `kind`, and a timestamp. Rows carry no owner: a post shows one row as its
//! cover through its own `cover_id`, and the library lists one tenant's rows
//! (ADR-0021). The page uploads through the app's own [`Uploader`] — the
//! `DirUploader` the panel installs for `FileUpload` — writes the row, and
//! lists what the library holds: a thumbnail for an image, a link for anything
//! else.

use std::collections::HashMap;

use tablo_core::{
    Notification, Uploader, csrf, db::db, notification::set_notification, require_tenant,
    schema::OptionSource,
};
use topcoat::{
    Result,
    asset::AssetConfig,
    context::{Cx, try_app_context},
    router::{
        content::multipart::Multipart,
        error::{SeeOther, bad_request, see_other},
        page, route,
    },
    view::{BoxView, View, ViewExt, attributes, view},
};

use crate::{
    app::{DirUploader, basename, upload_dir},
    models::MediaAsset,
};

/// Where the media library lives: the page, the upload route, and the form's
/// own action.
pub const MEDIA_PATH: &str = "/admin/media";

/// The widget script the page emits.
///
/// An app asset: ADR-0014's nine scripts are the shell's, and this one belongs
/// to the page that renders the widget. The page links it `defer`red, and only
/// when the router carries an asset bundle — the test router has none, like the
/// public blog's document.
pub const MEDIA_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/media.js");

/// The `kind` of a row whose bytes are an image: the row renders a thumbnail.
pub const KIND_IMAGE: &str = "image";

/// The `kind` of every other row: it renders a link.
pub const KIND_FILE: &str = "file";

/// The upload form's file field.
const FILE_FIELD: &str = "file";

/// The media library as a relationship source: one tenant's rows.
///
/// A post's cover picker loads its options through this source, so the tenant
/// gate and the tenant filter apply to the choice exactly as they apply to the
/// page that lists the same rows.
pub struct MediaLibrary;

impl OptionSource for MediaLibrary {
    type Model = MediaAsset;

    fn scoped_query(cx: &Cx) -> Result<toasty::stmt::Query<toasty::stmt::List<MediaAsset>>> {
        let tenant = require_tenant(cx)?;
        Ok(toasty::stmt::Query::<toasty::stmt::List<MediaAsset>>::all()
            .filter(MediaAsset::fields().tenant_id().eq(tenant)))
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn can_view(_cx: &Cx, _record: &MediaAsset) -> bool {
        true
    }

    fn requires_tenant() -> bool {
        true
    }

    fn slug() -> String {
        "media".to_string()
    }

    fn search_expr(_cx: &Cx, term: &str) -> Option<toasty::stmt::Expr<bool>> {
        let term = term.trim();
        if term.is_empty() {
            return None;
        }
        Some(
            MediaAsset::fields()
                .filename()
                .like_with_escape(format!("%{term}%"), '\\'),
        )
    }

    fn order_by(_cx: &Cx) -> Option<toasty::stmt::OrderByExpr> {
        Some(MediaAsset::fields().filename().asc())
    }
}

/// One media row's file: a thumbnail for an image, a link for anything else.
///
/// The framework's file field links every stored path the same way;
/// telling an image from the rest is the media library's job, and `kind` is
/// what the row recorded when the upload was stored. The public blog renders a
/// post's cover through this too, so one row looks the same wherever it is
/// shown.
pub fn media_file_view<'a>(cx: &'a Cx, asset: &MediaAsset) -> BoxView<'a> {
    if asset.kind == KIND_IMAGE {
        let src = asset.path.clone();
        let alt = asset.filename.clone();
        view! {
            cx =>
            <img
                src=(src)
                alt=(alt)
                data-media-thumbnail=""
                class="size-12 shrink-0 rounded-md border border-border object-cover"
            >
        }
        .boxed()
    } else {
        let href = asset.path.clone();
        let name = asset.filename.clone();
        view! {
            cx =>
            <a
                href=(href)
                data-media-link=""
                class="min-w-0 truncate text-sm underline"
            >
                (name)
            </a>
        }
        .boxed()
    }
}

/// The media library: every row this tenant holds, and the form that adds one.
///
/// The list is one query, not a paginated `Table`: a `Table` renders text
/// columns, and a thumbnail is not text. A library that outgrows one page wants
/// its own loader and pager, which is a different seam from this demo's.
#[page("/admin/media")]
async fn media_page(cx: &Cx) -> Result<impl View> {
    // One tenant's library: a tenantless request is refused rather
    // than served every tenant's rows.
    let tenant = require_tenant(cx)?;
    let mut db = db(cx);
    // No resource owns `MediaAsset`, so its tenant filter is this page's —
    // written once, on the column the model declares.
    let media = MediaAsset::filter(MediaAsset::fields().tenant_id().eq(tenant))
        .order_by(MediaAsset::fields().created_at().desc())
        .exec(&mut db)
        .await?;

    // The form is the app's, so the token is the app's to embed.
    let csrf_token = csrf::ensure_token(cx);
    let has_assets = try_app_context::<AssetConfig>(cx).is_some();

    Ok(view! {
        cx =>
        tablo_ui::page(
            tablo_ui::page_header(
                tablo_ui::page_title("Media library")
                tablo_ui::page_description(
                    "Files stored through the app's uploader, picked as post covers."
                )
            )
            tablo_ui::page_content(
                tablo_ui::card(
                    tablo_ui::card_header(tablo_ui::card_title("Upload"))
                    tablo_ui::card_content(
                        <form
                            method="post"
                            action=(MEDIA_PATH)
                            enctype="multipart/form-data"
                            class="flex flex-col gap-4"
                        >
                            (csrf::field(cx, &csrf_token))
                            <div class="flex flex-col gap-2">
                                <label class="text-sm font-medium" for="media-file">
                                    "File"
                                </label>
                                <div class="flex items-center gap-2">
                                    tablo_ui::input(
                                        attrs: attributes! {
                                            id="media-file"
                                            type="file"
                                            name=(FILE_FIELD)
                                            required=""
                                            data-media-file=""
                                        }
                                    )
                                    // The × is a reset control: with no script
                                    // the browser resets the form and the file
                                    // input empties; `media.js` empties the input
                                    // and the preview itself and cancels that
                                    // reset, so a file clear keeps the form
                                    // usable (ADR-0021).
                                    tablo_ui::button(
                                        variant: tablo_ui::ButtonVariant::Outline,
                                        size: tablo_ui::ButtonSize::Sm,
                                        attrs: attributes! {
                                            type="reset"
                                            data-media-clear=""
                                            aria-label="Clear the selected file"
                                        },
                                        "×"
                                    )
                                </div>
                                <div
                                    data-media-preview=""
                                    hidden=""
                                    class="flex items-center gap-3 text-xs text-muted-foreground"
                                ></div>
                            </div>
                            tablo_ui::button(
                                variant: tablo_ui::ButtonVariant::Primary,
                                attrs: attributes! { type="submit" },
                                "Upload"
                            )
                        </form>
                    )
                )
                tablo_ui::card(
                    tablo_ui::card_header(tablo_ui::card_title("Stored media"))
                    tablo_ui::card_content(
                        <div class="flex flex-col gap-3">
                            if media.is_empty() {
                                <p
                                    data-media-empty=""
                                    class="text-sm text-muted-foreground"
                                >
                                    "No media has been uploaded yet."
                                </p>
                            } else {
                                <ul
                                    data-media-list=""
                                    class="flex flex-col divide-y divide-border"
                                >
                                    for asset in &media {
                                        <li
                                            data-media-row=(asset.id.to_string())
                                            class="flex items-center gap-4 py-3"
                                        >
                                            (media_file_view(cx, asset))
                                            <div class="flex min-w-0 flex-col">
                                                if asset.kind == KIND_IMAGE {
                                                    <span class="truncate text-sm font-medium">
                                                        (asset.filename.clone())
                                                    </span>
                                                }
                                                <span class="text-xs text-muted-foreground">
                                                    (asset.created_at.strftime("%Y-%m-%d %H:%M").to_string())
                                                </span>
                                            </div>
                                        </li>
                                    }
                                </ul>
                            }
                        </div>
                    )
                )
                if has_assets {
                    <script src=(MEDIA_JS) defer=""></script>
                }
            )
        )
    })
}

/// `POST /admin/media` — store one uploaded file and write the row for it.
///
/// The page renders its own form, so it parses its own multipart body: the
/// framework's parser serves the fields a `Schema` declares, and this form is
/// not one. The bytes go through the app's own [`Uploader`] — the same
/// `DirUploader` the app gives `Panel::uploads` — outside any
/// transaction, like every upload (ADR-0017).
#[route(POST "/admin/media")]
async fn upload(cx: &Cx, mut multipart: Multipart) -> Result<SeeOther> {
    let tenant = require_tenant(cx)?;
    let mut values = HashMap::new();
    let mut file: Option<UploadedPart> = None;
    while let Some(field) = multipart.next_field().await? {
        let Some(name) = field.name().map(str::to_string) else {
            continue;
        };
        if name == FILE_FIELD {
            let filename = field.file_name().unwrap_or_default().to_string();
            let content_type = field.content_type().unwrap_or_default().to_string();
            let bytes = field.bytes().await?;
            file = Some(UploadedPart {
                filename,
                content_type,
                bytes: bytes.to_vec(),
            });
        } else {
            values.insert(name, field.text().await?);
        }
    }
    // The framework verifies the forms it renders; this one is the
    // app's, so the check is the app's too.
    csrf::verify(cx, &values)?;
    let part = file.ok_or_else(|| bad_request("Choose a file before uploading."))?;
    // The name the row records and the store writes: one rule, so the row's
    // `filename` and the file on disk cannot disagree.
    let filename = basename(&part.filename);
    if filename.is_empty() || part.bytes.is_empty() {
        return Err(bad_request("Choose a file before uploading.").into());
    }
    let mut db = db(cx);
    // The app's own store, pointed at the directory the panel serves: the
    // `Uploader` `Panel::uploads` installs lives on the app context for the
    // framework's form parser and is not readable from a page, so the page
    // builds the same store from the same configuration. What it returns is
    // the row's `path` verbatim — a URL that resolves back to these bytes.
    let path = DirUploader::new(upload_dir())
        .store(&filename, &part.bytes)
        .await
        .map_err(bad_request)?;
    toasty::create!(MediaAsset {
        tenant_id: tenant,
        path: path,
        filename: filename,
        kind: kind_of(&part.content_type).to_string(),
        created_at: jiff::Timestamp::now(),
    })
    .exec(&mut db)
    .await?;
    set_notification(cx, Notification::success("Media uploaded"));
    Ok(see_other(MEDIA_PATH))
}

/// The file part an upload form submitted, before anything is stored.
struct UploadedPart {
    filename: String,
    content_type: String,
    bytes: Vec<u8>,
}

/// Whether an uploaded part is an image, from the content type the browser sent
/// with it.
///
/// The framework's file field reads no extension and renders every stored path
/// the same way; deciding that a thumbnail suits *these* bytes is the
/// media library's, and the part's `Content-Type` is what the browser says they
/// are. It is a claim, not a sniff: a library that served those bytes to other
/// people would read their magic numbers instead.
fn kind_of(content_type: &str) -> &'static str {
    if content_type
        .trim()
        .to_ascii_lowercase()
        .starts_with("image/")
    {
        KIND_IMAGE
    } else {
        KIND_FILE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_follows_the_content_type() {
        assert_eq!(kind_of("image/png"), KIND_IMAGE);
        assert_eq!(kind_of("IMAGE/JPEG"), KIND_IMAGE);
        assert_eq!(kind_of(" text/plain"), KIND_FILE);
        assert_eq!(kind_of("application/pdf"), KIND_FILE);
        assert_eq!(kind_of(""), KIND_FILE);
    }
}
