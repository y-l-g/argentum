use jiff::Timestamp;
use toasty::Deferred;

/// The seeder and the demo constants live in the private `seed`
/// module; re-exported so the panel, the binary and the tests keep one import
/// path.
pub use crate::seed::{
    BLOCKED_TENANT, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, DEMO_TENANT, REMOVED_COMMENT_BODY,
    TENANTLESS_ADMIN_EMAIL, create_admin, seed, seed_content,
};

/// User shown in the admin list — the realistic spec model (US16):
/// role/active/created_at plus `#[index]` on the searchable `name` column.
/// `email` keeps only `#[unique]` — a unique constraint already implies an
/// index, and stacking `#[index]` on top would double it.
#[derive(Debug, Clone, toasty::Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub name: String,
    #[unique]
    pub email: String,
    /// "admin" or "member" — the form renders them as a static-options Select.
    pub role: String,
    pub active: bool,
    /// A stored integer the form binds through `TextInput::typed`: optional,
    /// zero or more.
    pub age: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, toasty::Model)]
// Scoped, not global: the form's unique probe runs through the
// tenant-scoped query (`scoped_query`), so a *global* unique index
// on `email` would be rejected by the database for an email another tenant
// already owns — after the probe passed — and surface as a 500. Constraining
// `(tenant_id, email)` makes the constraint say what the probe enforces, so two
// tenants may share an email.
#[unique(tenant_id, email)]
pub struct Author {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub tenant_id: uuid::Uuid,
    /// Display name.
    pub name: String,
    pub email: String,
    #[has_many]
    pub posts: Deferred<Vec<Post>>,
}

/// SEO metadata for a post — an embedded struct.
///
/// Flattens into the parent table as `seo_title` / `seo_description`: the same
/// row, no join, but two more columns the form binds like any other.
#[derive(Debug, Clone, toasty::Embed, tablo_core::EmbeddedForm)]
pub struct Seo {
    pub title: String,
    /// A multi-line control: the derive renders one `TextInput` per leaf, and
    /// this is the one leaf the panel wants as a `Textarea`.
    #[form(textarea, rows = 3)]
    pub description: String,
}

/// A post's lifecycle — an embedded enum whose **timestamps are shared**.
///
/// Every variant declares a timestamp under the same `#[shared(timestamp)]`
/// identifier, so the three coalesce into one `publication_timestamp` column
/// instead of one column per variant. The rest of each variant is its own
/// nullable column (`publication_scheduled_for`, `publication_canonical_url`,
/// `publication_reason`).
///
/// The timestamps are `jiff::Timestamp`: the form binds them through
/// `TextInput::typed`, which renders `type="datetime-local"` and reads the
/// submission back as UTC.
#[derive(Debug, Clone, PartialEq, toasty::Embed, tablo_core::EmbeddedForm)]
pub enum Publication {
    #[column(variant = 1)]
    Scheduled {
        /// The shared column's one control renders from the first variant that
        /// declares it, so its label is written there.
        #[shared(timestamp)]
        #[form(label = "Publication timestamp")]
        scheduled_at: Timestamp,
        scheduled_for: String,
    },
    #[column(variant = 2)]
    Published {
        #[shared(timestamp)]
        published_at: Timestamp,
        #[form(label = "Canonical URL")]
        canonical_url: String,
    },
    #[column(variant = 3)]
    Archived {
        #[shared(timestamp)]
        archived_at: Timestamp,
        #[form(label = "Archive reason")]
        reason: String,
    },
}

/// Post with BelongsTo Author and HasMany Comments (relations via include + computed).
#[derive(Debug, Clone, toasty::Model)]
pub struct Post {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub tenant_id: uuid::Uuid,
    #[index]
    pub title: String,
    pub body: String,
    #[index]
    pub status: String,
    pub featured: bool,
    pub created_at: Timestamp,
    /// The library row this post shows as its cover, if any. An optional
    /// single picker: the form offers the tenant's media rows and stores the
    /// picked row's id.
    pub cover_id: Option<uuid::Uuid>,
    pub tags: String,
    /// Embedded struct.
    pub seo: Seo,
    /// Shared column + per-variant payloads.
    pub publication: Publication,
    #[index]
    pub author_id: uuid::Uuid,
    #[belongs_to(key = author_id, references = id)]
    pub author: Deferred<Author>,
    #[has_many]
    pub comments: Deferred<Vec<Comment>>,
}

#[derive(Debug, Clone, toasty::Model)]
pub struct Comment {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    pub body: String,
    #[index]
    pub post_id: uuid::Uuid,
    #[belongs_to(key = post_id, references = id)]
    pub post: Deferred<Post>,
}

/// One stored file in the media library — the `medias` table.
///
/// A WordPress-style library: one row per stored file with the tenant that
/// uploaded it, the `path` the `Uploader` returned, the client's `filename`,
/// a `kind`, and a timestamp. Rows carry no owner: a post shows one row as
/// its cover through its own `cover_id`, and the library lists one tenant's
/// rows. ADR-0021 records the shape.
#[derive(Debug, Clone, toasty::Model)]
#[table = "medias"]
pub struct MediaAsset {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    /// The tenant that uploaded the file, like every other showcase row
    /// the library lists one tenant's media.
    #[index]
    pub tenant_id: uuid::Uuid,
    /// What the app's [`Uploader`](tablo_core::Uploader) returned, stored
    /// verbatim and rendered as the URL the file is served at.
    pub path: String,
    /// The client's filename, a basename, for display.
    pub filename: String,
    /// `"image"` or `"file"` — the app's own kind, from the uploaded part's
    /// content type. It decides whether a row renders a thumbnail or a link.
    pub kind: String,
    pub created_at: Timestamp,
}
