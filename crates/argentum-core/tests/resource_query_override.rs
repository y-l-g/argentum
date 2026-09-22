//! A `Resource::query` override scopes the rows the resource sees (GH #52,
//! #222).
//!
//! Both resources are hand-written: the `derive(Resource)` this file used to
//! exercise was removed as dead surface (GH #222), and the reference app
//! hand-writes its impls anyway. The coverage is unchanged — the trait's
//! default returns every row, and the override returns the scoped set.

use argentum_core::Resource;
use toasty::Db;
use topcoat::context::{Cx, CxTestBuilder};

#[derive(Debug, toasty::Model, Clone)]
struct User {
    #[key]
    #[auto]
    id: uuid::Uuid,
    name: String,
}

/// No `query` override: the trait default returns every row.
struct Everyone;

impl Resource for Everyone {
    type Model = User;
}

/// Overrides `query`: every load through this resource sees only Ada.
struct JustAda;

impl Resource for JustAda {
    type Model = User;

    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
        toasty::stmt::Query::<toasty::stmt::List<User>>::all()
            .filter(User::fields().name().eq("Ada"))
    }
}

#[tokio::test]
async fn query_override_scopes_rows() {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    toasty::create!(User {
        name: "Ada".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    toasty::create!(User {
        name: "Bob".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();

    let cx = CxTestBuilder::new().app_context(db).build();
    let mut db = argentum_core::db::db(&cx);

    let all = Everyone::query(&cx).exec(&mut db).await.unwrap();
    assert_eq!(all.len(), 2);

    let ada_only = JustAda::query(&cx).exec(&mut db).await.unwrap();
    assert_eq!(ada_only.len(), 1);
    assert_eq!(ada_only[0].name, "Ada");
}
