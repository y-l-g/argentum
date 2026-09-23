use jiff::Timestamp;
use toasty::Deferred;

/// The seeder and the demo constants (GH #87) live in the private `seed`
/// module; re-exported so the panel, the binary and the tests keep one import
/// path.
pub use crate::seed::{
    BLOCKED_TENANT, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, DEMO_TENANT, TENANTLESS_ADMIN_EMAIL,
    create_admin, seed, seed_phase2,
};

/// User shown in the admin list — the realistic spec model (US16, GH #13):
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
    /// "admin" or "member" — the form renders them as a static-options Select
    /// (GH #13).
    pub role: String,
    pub active: bool,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, toasty::Model)]
// Scoped, not global (GH #88): the form's unique probe runs through the
// tenant-scoped query (`scoped_query`, GH #223), so a *global* unique index
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

/// SEO metadata for a post — an embedded struct (GH #185).
///
/// Flattens into the parent table as `seo_title` / `seo_description`: the same
/// row, no join, but two more columns the form binds like any other.
#[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
pub struct Seo {
    pub title: String,
    /// A multi-line control: the derive renders one `TextInput` per leaf, and
    /// this is the one leaf the panel wants as a `Textarea` (GH #191).
    #[form(textarea, rows = 3)]
    pub description: String,
}

/// A post's lifecycle — an embedded enum whose **timestamps are shared**
/// (GH #185).
///
/// Every variant declares a timestamp under the same `#[shared(timestamp)]`
/// identifier, so the three coalesce into one `publication_timestamp` column
/// instead of one column per variant. The rest of each variant is its own
/// nullable column (`publication_scheduled_for`, `publication_canonical_url`,
/// `publication_reason`).
///
/// The timestamps are `String` rather than `jiff::Timestamp` because a bound
/// field is a `String` lens: the Schema's text fields accept `Path<M, String>`,
/// and a typed leaf (a timestamp, an integer) cannot bind as text yet. The
/// shared column's *coalescing* is what this demonstrates.
#[derive(Debug, Clone, PartialEq, toasty::Embed, argentum_core::EmbeddedForm)]
pub enum Publication {
    #[column(variant = 1)]
    Scheduled {
        /// The shared column's one control renders from the first variant that
        /// declares it, so its label is written there (GH #191).
        #[shared(timestamp)]
        #[form(label = "Publication timestamp")]
        scheduled_at: String,
        scheduled_for: String,
    },
    #[column(variant = 2)]
    Published {
        #[shared(timestamp)]
        published_at: String,
        #[form(label = "Canonical URL")]
        canonical_url: String,
    },
    #[column(variant = 3)]
    Archived {
        #[shared(timestamp)]
        archived_at: String,
        #[form(label = "Archive reason")]
        reason: String,
    },
}

/// Image / video attachment — an embedded enum with an embedded struct **nested
/// inside a variant** (GH #185).
///
/// `Video` carries a `Poster`, which itself embeds a `Credit`, so the column is
/// `media_poster_credit_author` — three levels deep, one flat column.
#[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
pub struct Credit {
    #[form(label = "Poster credit")]
    pub author: String,
    pub licence: String,
}

#[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
pub struct Poster {
    #[form(label = "Poster URL")]
    pub url: String,
    pub credit: Credit,
}

#[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
pub enum Media {
    #[column(variant = 1)]
    Image {
        #[form(label = "Image URL")]
        url: String,
        #[form(label = "Image alt")]
        alt: String,
    },
    #[column(variant = 2)]
    Video {
        // Distinct from `Image::url` on purpose: two variant fields mapping to
        // one column is a schema error unless they declare `#[shared(..)]` —
        // which is the right answer only when they mean the same thing.
        #[form(label = "Video URL")]
        video_url: String,
        poster: Poster,
    },
}

/// Post statistics — an embedded struct (GH #185), bindable since GH #192.
///
/// Embedding flattens it into `post_stats_word_count` /
/// `post_stats_read_minutes`, and those are integers the form binds through
/// `TextInput::typed`.
#[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
pub struct PostStats {
    #[form(label = "Word count")]
    pub word_count: i64,
    #[form(label = "Read minutes")]
    pub read_minutes: i64,
}

/// Post with BelongsTo Author and HasMany Comments (Phase 2 relations via include + computed).
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
    pub image_path: String,
    pub tags: String,
    /// Embedded struct (GH #185).
    pub seo: Seo,
    /// Shared column + per-variant payloads.
    pub publication: Publication,
    /// Embedded struct nested inside an enum variant.
    pub media: Media,
    pub post_stats: PostStats,
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

/// One stored file in the media library (GH #248) — the `medias` table.
///
/// **`owner_type`/`owner_id` are a polymorphic pair.** A media row names the
/// record it belongs to without a foreign key, because that record is a `Post`
/// or a `User` and Toasty's typed relations express one target table. So there
/// is no `#[belongs_to]` here and no `#[has_many]` on either owner: the
/// database enforces nothing, [`crate::media`] checks the owner exists before
/// it writes, and deleting an owner leaves its media rows dangling rather than
/// cascading. ADR-0021 records the tradeoff.
///
/// The name is `MediaAsset`, not `Media` or `Attachment`: `Media` is the
/// embedded enum on `Post` — an image/video *description* in the post's own
/// columns, with no bytes and no owner — and "Attachment" is only the label
/// over that value's controls in the post form. `CONTEXT.md` keeps the three
/// apart.
#[derive(Debug, Clone, toasty::Model)]
#[table = "medias"]
#[index(owner_type, owner_id)]
pub struct MediaAsset {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    /// The tenant that uploaded the file, like every other showcase row
    /// (GH #87): the library lists one tenant's media.
    #[index]
    pub tenant_id: uuid::Uuid,
    /// Which table `owner_id` names: [`crate::media::OWNER_POST`] or
    /// [`crate::media::OWNER_USER`].
    pub owner_type: String,
    pub owner_id: uuid::Uuid,
    /// What the app's [`Uploader`](argentum_core::Uploader) returned, stored
    /// verbatim and rendered as the URL the file is served at (GH #188).
    pub path: String,
    /// The client's filename, a basename, for display.
    pub filename: String,
    /// `"image"` or `"file"` — the app's own kind, from the uploaded part's
    /// content type. It decides whether a row renders a thumbnail or a link.
    pub kind: String,
    pub created_at: Timestamp,
}
