use jiff::Timestamp;
use toasty::Db;

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
