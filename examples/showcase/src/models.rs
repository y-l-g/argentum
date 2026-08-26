use argentum_core::resource::{GetField, HasId};
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

impl HasId for User {
    fn id_string(&self) -> String {
        self.id.to_string()
    }
}

impl GetField for User {
    fn get_field(&self, name: &str) -> String {
        match name {
            "name" => self.name.clone(),
            "email" => self.email.clone(),
            "id" => self.id.to_string(),
            _ => panic!(
                "GetField: unknown column '{}' for {}",
                name,
                std::any::type_name::<Self>()
            ),
        }
    }
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
