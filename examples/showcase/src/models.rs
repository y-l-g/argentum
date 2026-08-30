use toasty::Db;

/// User shown in the admin list.
///
/// `role` / `active` / `created_at` with `#[index]` (spec #6 US16) are
/// deferred until FilterBuilder needs them (see GH #13) — not needed for the
/// Table+Schema vertical slice and would force a migration + 3rd seed row now.
#[derive(Debug, Clone, toasty::Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    pub name: String,
    #[unique]
    pub email: String,
}

/// Seed a few users.
pub async fn seed(db: &mut Db) -> toasty::Result<()> {
    toasty::create!(User::[
        { name: "Ada Lovelace", email: "ada@example.com" },
        { name: "Grace Hopper", email: "grace@example.com" },
    ])
    .exec(db)
    .await?;
    Ok(())
}
