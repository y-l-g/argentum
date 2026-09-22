use jiff::Timestamp;
use toasty::{Db, Deferred};

use argentum_core::auth::{AdminUser, hash_password};

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
    /// "admin" or "member" — a string until Select fields land (GH #13).
    pub role: String,
    pub active: bool,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, toasty::Model)]
// Scoped, not global (GH #88): the form's unique probe runs through
// `AuthorResource::query`, which is tenant-filtered, so a *global* unique index
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
/// It was a `#[document]` until the typed binding landed: a document's inner
/// fields share its single JSON column, so no control could bind `word_count`
/// on its own — which is why the record fn parsed it with `unwrap_or(0)` and a
/// mistyped number became a silent zero. Embedding flattens it into
/// `post_stats_word_count` / `post_stats_read_minutes`, and those are integers
/// the form binds through `TextInput::typed`.
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
    /// Embedded struct + nested document (GH #185).
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

/// Seed the team roster. Names sort deterministically (name-asc): Ada and
/// Alan stay first for pagination and search tests, followed by six more
/// engineers.
pub async fn seed(db: &mut Db) -> toasty::Result<()> {
    toasty::create!(User::[
        {
            name: "Ada Lovelace",
            email: "ada@example.com",
            role: "admin",
            active: true,
            created_at: "2024-01-15T09:30:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Alan Turing",
            email: "alan@example.com",
            role: "member",
            active: false,
            created_at: "2024-06-01T12:00:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Grace Hopper",
            email: "grace@example.com",
            role: "member",
            active: true,
            created_at: "2023-11-20T18:45:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Claude Shannon",
            email: "claude@example.com",
            role: "member",
            active: true,
            created_at: "2024-02-10T10:00:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Dorothy Vaughan",
            email: "dorothy@example.com",
            role: "member",
            active: true,
            created_at: "2024-02-18T14:00:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Edsger Dijkstra",
            email: "edsger@example.com",
            role: "member",
            active: false,
            created_at: "2024-03-05T09:00:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Frances Allen",
            email: "frances@example.com",
            role: "admin",
            active: true,
            created_at: "2024-03-12T16:30:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
        {
            name: "Ken Thompson",
            email: "ken@example.com",
            role: "member",
            active: true,
            created_at: "2024-04-02T11:15:00Z"
                .parse::<Timestamp>()
                .expect("timestamp"),
        },
    ])
    .exec(db)
    .await?;
    create_admin(
        db,
        DEMO_ADMIN_EMAIL,
        "Demo Admin",
        DEMO_ADMIN_PASSWORD,
        Some(DEMO_TENANT),
    )
    .await?;
    create_admin(
        db,
        TENANTLESS_ADMIN_EMAIL,
        "No Tenant",
        DEMO_ADMIN_PASSWORD,
        None,
    )
    .await?;
    Ok(())
}

/// The tenant owning all showcase seed rows (GH #87): seeds never mint
/// nil-tenant orphans, and the demo admin owns it.
pub const DEMO_TENANT: uuid::Uuid = uuid::Uuid::from_u128(100);

/// A deterministic id for the `index`th seeded post (GH #184).
///
/// `PostResource` declares no sortable column, so a paginated list falls back
/// to primary-key order. With `#[auto]` ids that order is effectively random,
/// which put the rows the showcase pins — "Hello Toasty" and "Second Post" —
/// wherever a UUID hash happened to land once the seed outgrew one page.
/// Pinning the six narrative rows first and the pagination filler after them
/// (`FILLER_ID_BASE`) makes page 1 deterministic: the stories, then filler.
fn seeded_post_id(index: usize) -> uuid::Uuid {
    uuid::Uuid::from_u128(index as u128)
}

/// First id handed to a pagination filler row (GH #184): above `7fff…`, which
/// is outside the range a random v4 UUID practically lands in, so the filler
/// always sorts after the narrative rows above.
const FILLER_ID_BASE: u128 = 0x8000_0000_0000_0000_0000_0000_0000_0000;

/// Backlog titles for the pagination fixture (GH #184): sixty drafts, which
/// with the six rows above and `PostResource`'s page size of 10 gives seven
/// pages — enough to walk forward, walk back, and land mid-list.
///
/// A `const` with a compile-time length assertion, so shrinking it below a
/// few pages fails the build rather than silently removing the demo.
const PAGINATION_FILLER_TITLES: [&str; 60] = [
    "Cursor Pagination, Explained Slowly",
    "What We Learned From a 500-Row Admin Table",
    "Indexing the Columns Editors Actually Sort By",
    "A Draft Is Not a Todo",
    "Why Our Search Matches Substrings",
    "Escaping User Input in a LIKE Predicate",
    "The Case Against Infinite Scroll in Admin Tools",
    "Reading Query Plans Without Panicking",
    "Tenant Scoping Is a Query, Not a Filter",
    "One Round-Trip: Preloading Relations",
    "When a Cache Hides a Fresh Write",
    "Modelling Drafts, Scheduled Posts, and Retractions",
    "Why the Panel Owns Its Stylesheet",
    "Server-Rendered Forms Without a Client Framework",
    "Morphing a Table In Place",
    "Keeping Focus Across a Live Re-render",
    "The Cost of a Second Database Round-Trip",
    "Bulk Actions Need Confirmation, Not Optimism",
    "Naming Things in an Admin Panel",
    "Why Deletes Are Record Functions",
    "Authorization Belongs in the Handler",
    "Policy Checks Are Not Middleware",
    "What a Row Key Is For",
    "Display Keys Are Not Primary Keys",
    "Grouping Counts Are Page-Local",
    "Exporting a Filtered View",
    "RFC 4180 and the Humble CSV",
    "File Uploads Without a Bucket",
    "Storing a Filename Is Not Storing a File",
    "Required Fields and the Empty Submit",
    "Validation Errors Belong Next to the Field",
    "Accessible Forms for an Internal Tool",
    "Keyboard-First Admin Work",
    "Dark Mode Without a Flash of Wrong Theme",
    "Tokens Over Hard-Coded Colors",
    "Owning Your Primitives",
    "Syncing Components Without Forking Them",
    "A Sidebar That Remembers Itself",
    "Toasts That Do Not Steal Focus",
    "Empty States Are Not Errors",
    "Failed Loads Need a Retry",
    "Streaming a Skeleton Before the Rows",
    "Cursor Pagination Beats Offset at Scale",
    "Stable Sort Orders for Stable Pages",
    "What Happens When the Cursor Goes Stale",
    "Escaping the Search Term",
    "Trimming Before Validating",
    "Absent Keys Mean Unchanged",
    "Repeaters and Partial Groups",
    "Relationships in a Select",
    "Too Many Options to Load",
    "Searching Options on the Server",
    "Selecting a Variant",
    "Three Ways to Filter a List",
    "Filter State Lives in the URL",
    "Deep Links Into a Filtered Table",
    "Testing an Admin Panel Over HTTP",
    "Fixtures That Do Not Lie",
    "Benchmarking What Users Feel",
    "Writing Down the Decisions",
];
/// A tenant whose Policy denies everything: the legible deny path for
/// tenancy tests (no magic values at call sites).
pub const BLOCKED_TENANT: uuid::Uuid = uuid::Uuid::from_u128(9999);

/// Demo administrator credentials, shown on the login page and in the README.
pub const DEMO_ADMIN_EMAIL: &str = "admin@example.com";
pub const DEMO_ADMIN_PASSWORD: &str = "password";

/// A seeded administrator with no tenant, for `requires_tenant` fail-closed
/// tests: valid credentials, no tenant to bridge.
pub const TENANTLESS_ADMIN_EMAIL: &str = "root@example.com";

/// Create an active admin (or another app user) with an Argon2id-hashed
/// password. Used by the showcase seed and the tenancy test fixtures.
pub async fn create_admin(
    db: &mut Db,
    email: &str,
    display_name: &str,
    password: &str,
    tenant_id: Option<uuid::Uuid>,
) -> toasty::Result<AdminUser> {
    toasty::create!(AdminUser {
        email: email.to_string(),
        password_hash: hash_password(password).expect("hash a demo password"),
        display_name: display_name.to_string(),
        active: true,
        tenant_id,
        created_at: Timestamp::now(),
    })
    .exec(db)
    .await
}

/// The embedded shapes a filler/backlog row carries (GH #185).
///
/// A compact, valid default so the pagination filler does not repeat four
/// nested literals sixty times. The narrative rows below spell theirs out, so
/// the showcase has real embedded data to look at.
fn filler_embedded(index: usize) -> (Seo, Publication, Media, PostStats) {
    (
        Seo {
            title: String::new(),
            description: String::new(),
        },
        Publication::Scheduled {
            scheduled_at: String::new(),
            scheduled_for: String::new(),
        },
        Media::Image {
            url: format!("backlog-{index:02}.jpg"),
            alt: String::new(),
        },
        PostStats {
            word_count: 0,
            read_minutes: 0,
        },
    )
}

/// Seed Phase 2 relation data (Authors + Posts + Comments) — call only when DB was built with all models.
///
/// The two original rows keep their identity (filter/group/export tests pin
/// them); the four extra posts are drafts with `featured = false` so the
/// published/featured filter assertions keep holding while the list shows a
/// believable backlog.
///
/// GH #184 added the pagination filler below, which puts the list well past one
/// page. Assertions that need a specific row now narrow by `?q=` rather than
/// assuming it is on the title-ordered first page, and "nothing was created"
/// assertions compare a before/after count instead of a literal seed size.
pub async fn seed_phase2(db: &mut Db) -> toasty::Result<()> {
    // Authors
    if Author::all().exec(db).await?.is_empty() {
        let tenant = DEMO_TENANT;
        let ada_author = toasty::create!(Author {
            tenant_id: tenant,
            name: "Ada Author",
            email: "ada.author@example.com",
        })
        .exec(db)
        .await?;
        let alan_author = toasty::create!(Author {
            tenant_id: tenant,
            name: "Alan Author",
            email: "alan.author@example.com",
        })
        .exec(db)
        .await?;
        let june_writer = toasty::create!(Author {
            tenant_id: tenant,
            name: "June Writer",
            email: "june.writer@example.com",
        })
        .exec(db)
        .await?;
        let rosa_editor = toasty::create!(Author {
            tenant_id: tenant,
            name: "Rosa Editor",
            email: "rosa.editor@example.com",
        })
        .exec(db)
        .await?;
        toasty::create!(Post {
            id: seeded_post_id(0),
            tenant_id: tenant,
            title: "Hello Toasty",
            body: "How we render admin tables over Toasty queries without an N+1.",
            status: "published".to_string(),
            featured: true,
            created_at: "2024-01-15T09:30:00Z".parse::<Timestamp>().unwrap(),
            image_path: "hello-toasty.jpg".to_string(),
            tags: "rust,async".to_string(),
            // Embedded shapes with real values (GH #185): an embedded struct,
            // an embedded enum whose timestamps share one column, an embedded
            // struct nested inside a variant, and a document.
            seo: Seo {
                title: "Hello Toasty — the admin panel".to_string(),
                description: "How we render admin tables over Toasty.".to_string(),
            },
            publication: Publication::Published {
                published_at: "2024-01-15T09:30:00Z".to_string(),
                canonical_url: "https://example.com/hello-toasty".to_string(),
            },
            media: Media::Video {
                video_url: "hello-toasty.mp4".to_string(),
                poster: Poster {
                    url: "hello-toasty-poster.jpg".to_string(),
                    credit: Credit {
                        author: "Ada Author".to_string(),
                        licence: "CC-BY-4.0".to_string(),
                    },
                },
            },
            post_stats: PostStats {
                word_count: 1240,
                read_minutes: 6,
            },
            author_id: ada_author.id,
        })
        .exec(db)
        .await?;
        toasty::create!(Post {
            id: seeded_post_id(1),
            tenant_id: tenant,
            title: "Second Post",
            body: "Draft notes on cursor pagination edge cases.",
            status: "draft".to_string(),
            featured: false,
            created_at: "2024-06-01T12:00:00Z".parse::<Timestamp>().unwrap(),
            image_path: "second-post.jpg".to_string(),
            tags: "draft".to_string(),
            seo: Seo {
                title: "Second Post".to_string(),
                description: "Draft notes on cursor pagination.".to_string(),
            },
            publication: Publication::Scheduled {
                scheduled_at: "2024-07-01T09:00:00Z".to_string(),
                scheduled_for: "2024-08-01T09:00:00Z".to_string(),
            },
            media: Media::Image {
                url: "second-post.jpg".to_string(),
                alt: "A draft cover".to_string(),
            },
            post_stats: PostStats {
                word_count: 320,
                read_minutes: 2,
            },
            author_id: alan_author.id,
        })
        .exec(db)
        .await?;
        for (index, (title, body, tags, created_at, author_id)) in [
            (
                "Row-Level Caching Notes",
                "When the list cache helps and when it hides fresh writes.",
                "performance,caching",
                "2024-02-08T10:00:00Z",
                june_writer.id,
            ),
            (
                "Async Rust Patterns",
                "A field guide to the executors and channels we actually use.",
                "rust,async",
                "2024-02-20T15:30:00Z",
                rosa_editor.id,
            ),
            (
                "Reviewing Query Plans",
                "Reading EXPLAIN output before reaching for an index.",
                "database,sql",
                "2024-03-14T09:45:00Z",
                june_writer.id,
            ),
            (
                "Onboarding Runbook",
                "Checklist for bringing a new editor onto the panel.",
                "process,docs",
                "2024-04-09T13:20:00Z",
                rosa_editor.id,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (seo, publication, media, post_stats) = filler_embedded(index);
            toasty::create!(Post {
                id: seeded_post_id(index + 2),
                tenant_id: tenant,
                title: title,
                body: body,
                status: "draft".to_string(),
                featured: false,
                created_at: created_at.parse::<Timestamp>().unwrap(),
                image_path: "draft-cover.jpg".to_string(),
                tags: tags,
                seo: seo,
                publication: publication,
                media: media,
                post_stats: post_stats,
                author_id: author_id,
            })
            .exec(db)
            .await?;
        }
        // Pagination filler (GH #184): `PostResource` paginates at 10, so the
        // six rows above could never cross a page boundary and the pager had
        // no demo path at all. These are drafts with `featured = false`, which
        // keeps every published/featured assertion holding.
        //
        // Deterministic on purpose: titles are a fixed ordered list, ids and
        // `created_at` are both derived from the index.
        //
        // The ids are pinned high on purpose. `PostResource` declares no
        // sortable column, so a paginated list falls back to primary-key order
        // ascending — and the two original rows above carry random UUID keys.
        // Filler past `7fff…` therefore widens the list behind them instead of
        // shuffling them off page 1, which is what the pinned assertions in
        // `filter_check` / `group_export_check` / `relation_check` rely on.
        for (index, title) in PAGINATION_FILLER_TITLES.iter().enumerate() {
            let created_at = jiff::civil::date(2024, 7, 1)
                .at(9, 0, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .expect("a valid zoned time")
                .checked_add(jiff::Span::new().hours(index as i64 * 6))
                .expect("a representable timestamp")
                .timestamp();
            let author_id = match index % 4 {
                0 => ada_author.id,
                1 => alan_author.id,
                2 => june_writer.id,
                _ => rosa_editor.id,
            };
            let (seo, publication, media, post_stats) = filler_embedded(index);
            toasty::create!(Post {
                id: uuid::Uuid::from_u128(FILLER_ID_BASE + index as u128),
                tenant_id: tenant,
                title: *title,
                body: "Backlog draft kept for pagination coverage.",
                status: "draft".to_string(),
                featured: false,
                created_at: created_at,
                image_path: "draft-cover.jpg".to_string(),
                tags: "backlog,draft".to_string(),
                seo: seo,
                publication: publication,
                media: media,
                post_stats: post_stats,
                author_id: author_id,
            })
            .exec(db)
            .await?;
        }
        let first_post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
            .first()
            .exec(db)
            .await?
            .expect("post");
        for body in [
            "Clear write-up — the include strategy finally clicked.",
            "Tried this on our staging data, pagination stayed stable.",
            "Small nit: the CSV export section deserves its own post.",
        ] {
            toasty::create!(Comment {
                body: body,
                post_id: first_post.id,
            })
            .exec(db)
            .await?;
        }
    }
    Ok(())
}
