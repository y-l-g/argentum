//! The media library (GH #248): the `medias` table, the page that fills it,
//! and the widget that previews a file before it is stored.
//!
//! A media row is a stored file plus a **polymorphic owner**: `owner_type` and
//! `owner_id` name a `Post` or a `User`, and no foreign key can hold that pair
//! together (ADR-0021). The page uploads through the app's own [`Uploader`] —
//! the `DirUploader` the panel installs for `FileUpload` (GH #188) — writes the
//! row, and lists what the library holds: a thumbnail for an image, a link for
//! anything else.
//!
//! `Post::media` is a different thing and keeps its name: the embedded enum
//! holding an image/video *description* in the post's own columns. This page
//! stores files; that value describes one. `CONTEXT.md` keeps the two apart.

use std::collections::HashMap;

use argentum_core::{
    Notification, Resource, Uploader, csrf, db::db, notification::set_notification, require_tenant,
    scoped_query,
};
use toasty::Db;
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
    app::{DirUploader, PostResource, UserResource, basename, upload_dir},
    models::{MediaAsset, Post, User},
};

/// Where the media library lives: the page, the upload route, and the form's
/// own action.
pub const MEDIA_PATH: &str = "/admin/media";

/// The widget script the page emits (GH #248).
///
/// An app asset: ADR-0014's nine scripts are the shell's, and this one belongs
/// to the page that renders the widget. The page links it `defer`red, and only
/// when the router carries an asset bundle — the test router has none, like the
/// public blog's document.
pub const MEDIA_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/media.js");

/// The `owner_type` a post-owned media row stores.
pub const OWNER_POST: &str = "post";

/// The `owner_type` a user-owned media row stores.
pub const OWNER_USER: &str = "user";

/// The `kind` of a row whose bytes are an image: the row renders a thumbnail.
pub const KIND_IMAGE: &str = "image";

/// The `kind` of every other row: it renders a link.
pub const KIND_FILE: &str = "file";

/// The upload form's owner field.
const OWNER_FIELD: &str = "owner";

/// The upload form's file field.
const FILE_FIELD: &str = "file";

/// The record a media row belongs to (GH #248): the typed half of the
/// polymorphic pair.
///
/// One column pair cannot name two tables, so `MediaAsset` stores the kind and
/// the key as a string and a `Uuid`, and this enum is where the app recovers
/// the type it lost. The two variants are the owners the showcase attaches
/// media to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOwner {
    Post(uuid::Uuid),
    User(uuid::Uuid),
}

impl MediaOwner {
    /// The `owner_type` this owner stores.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Post(_) => OWNER_POST,
            Self::User(_) => OWNER_USER,
        }
    }

    /// The `owner_id` this owner stores.
    pub fn id(self) -> uuid::Uuid {
        match self {
            Self::Post(id) | Self::User(id) => id,
        }
    }

    /// The form value naming this owner: `"post:<uuid>"` or `"user:<uuid>"`.
    pub fn value(self) -> String {
        format!("{}:{}", self.kind(), self.id())
    }

    /// Read a form value back, or `None` when it names no owner this library
    /// stores.
    pub fn parse(value: &str) -> Option<Self> {
        let (kind, id) = value.trim().split_once(':')?;
        let id = id.parse::<uuid::Uuid>().ok()?;
        match kind {
            OWNER_POST => Some(Self::Post(id)),
            OWNER_USER => Some(Self::User(id)),
            _ => None,
        }
    }
}

/// The media rows attached to `owner` (GH #248).
///
/// This is the whole relation the polymorphic pair buys: no foreign key, no
/// `#[has_many]`, one equality filter on the pair. A row whose owner was
/// deleted still matches, which is the tradeoff ADR-0021 records — the caller
/// decides what to do about it.
pub async fn media_for_owner(db: &mut Db, owner: MediaOwner) -> toasty::Result<Vec<MediaAsset>> {
    MediaAsset::filter(
        MediaAsset::fields()
            .owner_type()
            .eq(owner.kind().to_string())
            .and(MediaAsset::fields().owner_id().eq(owner.id())),
    )
    .order_by(MediaAsset::fields().created_at().desc())
    .exec(db)
    .await
}

/// One media row's file: a thumbnail for an image, a link for anything else
/// (GH #248).
///
/// The framework's file field links every stored path the same way (GH #242);
/// telling an image from the rest is the media library's job, and `kind` is
/// what the row recorded when the upload was stored. The public blog renders a
/// post's media through this too, so one row looks the same wherever it is
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
    // One tenant's library (GH #87): a tenantless request is refused rather
    // than served every tenant's rows.
    let tenant = require_tenant(cx)?;
    let mut db = db(cx);
    // The picker's owners. Posts go through the tenant-scoped query the panel
    // uses (GH #223), so it cannot offer another tenant's post; the showcase's
    // users are global.
    let posts = scoped_query::<PostResource>(cx)?
        .order_by(Post::fields().title().asc())
        .exec(&mut db)
        .await?;
    let users = UserResource::query(cx)
        .order_by(User::fields().name().asc())
        .exec(&mut db)
        .await?;
    // No resource owns `MediaAsset`, so its tenant filter is this page's —
    // written once, on the column the model declares.
    let media = MediaAsset::filter(MediaAsset::fields().tenant_id().eq(tenant))
        .order_by(MediaAsset::fields().created_at().desc())
        .exec(&mut db)
        .await?;

    let post_titles: HashMap<uuid::Uuid, String> = posts
        .iter()
        .map(|post| (post.id, post.title.clone()))
        .collect();
    let user_names: HashMap<uuid::Uuid, String> = users
        .iter()
        .map(|user| (user.id, user.name.clone()))
        .collect();

    // The form is the app's, so the token is the app's to embed (GH #99).
    let csrf_token = csrf::ensure_token(cx);
    let has_assets = try_app_context::<AssetConfig>(cx).is_some();

    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Media library")
                argentum_ui::page_description(
                    "Files stored through the app's uploader, attached to a post or a user."
                )
            )
            argentum_ui::page_content(
                argentum_ui::card(
                    argentum_ui::card_header(argentum_ui::card_title("Upload"))
                    argentum_ui::card_content(
                        <form
                            method="post"
                            action=(MEDIA_PATH)
                            enctype="multipart/form-data"
                            class="flex flex-col gap-4"
                        >
                            (csrf::field(cx, &csrf_token))
                            <div class="flex flex-col gap-2">
                                <label class="text-sm font-medium" for="media-owner">
                                    "Owner"
                                </label>
                                argentum_ui::select(
                                    attrs: attributes! { id="media-owner" name=(OWNER_FIELD) required="" },
                                    <option value="" selected="">"Choose an owner…"</option>
                                    <optgroup label="Posts">
                                        for post in &posts {
                                            <option value=(MediaOwner::Post(post.id).value())>
                                                (post.title.clone())
                                            </option>
                                        }
                                    </optgroup>
                                    <optgroup label="Users">
                                        for user in &users {
                                            <option value=(MediaOwner::User(user.id).value())>
                                                (user.name.clone())
                                            </option>
                                        }
                                    </optgroup>
                                )
                            </div>
                            <div class="flex flex-col gap-2">
                                <label class="text-sm font-medium" for="media-file">
                                    "File"
                                </label>
                                <div class="flex items-center gap-2">
                                    argentum_ui::input(
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
                                    // reset, so a file clear keeps the owner the
                                    // user picked (ADR-0021).
                                    argentum_ui::button(
                                        variant: argentum_ui::ButtonVariant::Outline,
                                        size: argentum_ui::ButtonSize::Sm,
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
                            argentum_ui::button(
                                variant: argentum_ui::ButtonVariant::Primary,
                                attrs: attributes! { type="submit" },
                                "Upload"
                            )
                        </form>
                    )
                )
                argentum_ui::card(
                    argentum_ui::card_header(argentum_ui::card_title("Stored media"))
                    argentum_ui::card_content(
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
                                                    (owner_label(asset, &post_titles, &user_names))
                                                </span>
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

/// `POST /admin/media` — store one uploaded file and write the row that owns it
/// (GH #248).
///
/// The page renders its own form, so it parses its own multipart body: the
/// framework's parser serves the fields a `Schema` declares, and this form is
/// not one. The bytes go through the app's own [`Uploader`] — the same
/// `DirUploader` the app gives `Panel::uploads` (GH #188) — outside any
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
    // The framework verifies the forms it renders (GH #99); this one is the
    // app's, so the check is the app's too.
    csrf::verify(cx, &values)?;
    let owner = values
        .get(OWNER_FIELD)
        .and_then(|value| MediaOwner::parse(value))
        .ok_or_else(|| bad_request("Choose an owner before uploading."))?;
    let part = file.ok_or_else(|| bad_request("Choose a file before uploading."))?;
    // The name the row records and the store writes: one rule, so the row's
    // `filename` and the file on disk cannot disagree (GH #90).
    let filename = basename(&part.filename);
    if filename.is_empty() || part.bytes.is_empty() {
        return Err(bad_request("Choose a file before uploading.").into());
    }
    let mut db = db(cx);
    // A polymorphic pair carries no foreign key, so nothing else can tell a
    // dangling owner from a real one: the app's check is the integrity the
    // database does not have (ADR-0021).
    if !owner_exists(cx, owner, &mut db).await? {
        return Err(bad_request("That owner does not exist.").into());
    }
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
        owner_type: owner.kind().to_string(),
        owner_id: owner.id(),
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

/// Whether `owner` names a record this app attaches media to (GH #248).
///
/// A post is resolved through the tenant-scoped query the panel uses
/// (GH #223), so a post in another tenant is not an owner here; a user is
/// resolved globally, because the showcase's users carry no tenant. The row's
/// own tenant is the uploader's, checked by the page that lists it.
async fn owner_exists(cx: &Cx, owner: MediaOwner, db: &mut Db) -> Result<bool> {
    let exists = match owner {
        MediaOwner::Post(id) => scoped_query::<PostResource>(cx)?
            .filter(Post::fields().id().eq(id))
            .first()
            .exec(&mut *db)
            .await?
            .is_some(),
        MediaOwner::User(id) => User::filter(User::fields().id().eq(id))
            .first()
            .exec(&mut *db)
            .await?
            .is_some(),
    };
    Ok(exists)
}

/// The owner a row names, as the page shows it.
///
/// A row whose owner is gone still renders — the pair has no foreign key, so
/// deleting a post or a user leaves its media behind — and says so rather than
/// showing a blank (ADR-0021).
fn owner_label(
    asset: &MediaAsset,
    post_titles: &HashMap<uuid::Uuid, String>,
    user_names: &HashMap<uuid::Uuid, String>,
) -> String {
    match asset.owner_type.as_str() {
        OWNER_POST => match post_titles.get(&asset.owner_id) {
            Some(title) => format!("Post · {title}"),
            None => "Post · (deleted)".to_string(),
        },
        OWNER_USER => match user_names.get(&asset.owner_id) {
            Some(name) => format!("User · {name}"),
            None => "User · (deleted)".to_string(),
        },
        other => format!("{other} · (unknown owner kind)"),
    }
}

/// Whether an uploaded part is an image, from the content type the browser sent
/// with it (GH #248).
///
/// The framework's file field reads no extension and renders every stored path
/// the same way (GH #242); deciding that a thumbnail suits *these* bytes is the
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
    fn a_form_value_round_trips_through_the_owner_pair() {
        let post = uuid::Uuid::from_u128(7);
        let owner = MediaOwner::Post(post);
        assert_eq!(owner.value(), format!("post:{post}"));
        assert_eq!(MediaOwner::parse(&owner.value()), Some(owner));
        assert_eq!(owner.kind(), OWNER_POST);
        assert_eq!(owner.id(), post);

        let user = uuid::Uuid::from_u128(8);
        let owner = MediaOwner::User(user);
        assert_eq!(MediaOwner::parse(&owner.value()), Some(owner));
        assert_eq!(owner.kind(), OWNER_USER);
    }

    #[test]
    fn a_value_naming_no_owner_is_refused() {
        for value in ["", "post", "post:", ":7", "author:7", "post:not-a-uuid"] {
            assert_eq!(MediaOwner::parse(value), None, "{value:?}");
        }
    }

    #[test]
    fn the_kind_follows_the_content_type() {
        assert_eq!(kind_of("image/png"), KIND_IMAGE);
        assert_eq!(kind_of("IMAGE/JPEG"), KIND_IMAGE);
        assert_eq!(kind_of(" text/plain"), KIND_FILE);
        assert_eq!(kind_of("application/pdf"), KIND_FILE);
        assert_eq!(kind_of(""), KIND_FILE);
    }
}
