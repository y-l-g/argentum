use jiff::Timestamp;
use toasty::{Db, Deferred};

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
    /// Display name.
    pub name: String,
    #[unique]
    pub email: String,
    #[has_many]
    pub posts: Deferred<Vec<Post>>,
}

/// Post with BelongsTo Author (Phase 2 relation via include + computed column).
#[derive(Debug, Clone, toasty::Model)]
pub struct Post {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub title: String,
    pub body: String,
    #[index]
    pub author_id: uuid::Uuid,
    #[belongs_to(key = author_id, references = id)]
    pub author: Deferred<Author>,
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

/// Seed a few users. Names are chosen so the default `name`-asc sort has a
/// deterministic order: Ada Lovelace, Alan Turing, Grace Hopper.
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
    ])
    .exec(db)
    .await?;
    Ok(())
}

/// Seed Phase 2 relation data (Authors + Posts + Comments) — call only when DB was built with all models.
pub async fn seed_phase2(db: &mut Db) -> toasty::Result<()> {
    // Authors
    if Author::all().exec(db).await?.is_empty() {
        let ada_author = toasty::create!(Author {
            name: "Ada Author",
            email: "ada.author@example.com",
        })
        .exec(db)
        .await?;
        let alan_author = toasty::create!(Author {
            name: "Alan Author",
            email: "alan.author@example.com",
        })
        .exec(db)
        .await?;
        toasty::create!(Post {
            title: "Hello Toasty",
            body: "First post body",
            author_id: ada_author.id,
        })
        .exec(db)
        .await?;
        toasty::create!(Post {
            title: "Second Post",
            body: "More content",
            author_id: alan_author.id,
        })
        .exec(db)
        .await?;
        let first_post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
            .first()
            .exec(db)
            .await?
            .expect("post");
        toasty::create!(Comment {
            body: "Nice post!",
            post_id: first_post.id,
        })
        .exec(db)
        .await?;
    }
    Ok(())
}
