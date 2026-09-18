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
pub struct Author {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub tenant_id: uuid::Uuid,
    /// Display name.
    pub name: String,
    #[unique]
    pub email: String,
    #[has_many]
    pub posts: Deferred<Vec<Post>>,
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

/// Seed the team roster. Names sort deterministically (name-asc): the
/// original Ada/Alan/Grace trio stays first for pagination and search tests,
/// followed by five more engineers.
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

/// Seed Phase 2 relation data (Authors + Posts + Comments) — call only when DB was built with all models.
///
/// The two original rows stay stable (filter/group/export tests pin them);
/// the four extra posts are drafts with featured=false so the
/// published/featured filter assertions keep holding while the list shows a
/// believable backlog.
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
            tenant_id: tenant,
            title: "Hello Toasty",
            body: "How we render admin tables over Toasty queries without an N+1.",
            status: "published".to_string(),
            featured: true,
            created_at: "2024-01-15T09:30:00Z".parse::<Timestamp>().unwrap(),
            image_path: "hello-toasty.jpg".to_string(),
            tags: "rust,async".to_string(),
            author_id: ada_author.id,
        })
        .exec(db)
        .await?;
        toasty::create!(Post {
            tenant_id: tenant,
            title: "Second Post",
            body: "Draft notes on cursor pagination edge cases.",
            status: "draft".to_string(),
            featured: false,
            created_at: "2024-06-01T12:00:00Z".parse::<Timestamp>().unwrap(),
            image_path: "second-post.jpg".to_string(),
            tags: "draft".to_string(),
            author_id: alan_author.id,
        })
        .exec(db)
        .await?;
        for (title, body, tags, created_at, author_id) in [
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
        ] {
            toasty::create!(Post {
                tenant_id: tenant,
                title: title,
                body: body,
                status: "draft".to_string(),
                featured: false,
                created_at: created_at.parse::<Timestamp>().unwrap(),
                image_path: "draft-cover.jpg".to_string(),
                tags: tags,
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
