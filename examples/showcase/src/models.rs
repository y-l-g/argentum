use toasty::Db;

/// User shown in the admin list.
#[derive(Debug, toasty::Model)]
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
