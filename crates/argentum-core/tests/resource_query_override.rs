use argentum_core::{IncludeNeeds, Resource};
use toasty::Db;
use topcoat::context::{Cx, CxTestBuilder};

fn scoped(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
    toasty::stmt::Query::<toasty::stmt::List<User>>::all().filter(User::fields().name().eq("Ada"))
}

/// GH #177: the export's base query, narrowed to what the columns declared.
fn export_scoped(_cx: &Cx, needs: &IncludeNeeds) -> toasty::stmt::Query<toasty::stmt::List<User>> {
    let all = toasty::stmt::Query::<toasty::stmt::List<User>>::all();
    if needs.wants("ada") {
        all.filter(User::fields().name().eq("Ada"))
    } else {
        all
    }
}

#[derive(Debug, toasty::Model, Clone)]
struct User {
    #[key]
    #[auto]
    id: uuid::Uuid,
    name: String,
}

#[derive(Resource)]
#[resource(model = User)]
struct Everyone;

#[derive(Resource)]
#[resource(model = User, query = scoped)]
struct JustAda;

/// The narrowed half is independently optional (GH #177): a derived resource
/// can declare `export_query` without re-declaring `query`, and keeps the
/// inherited full one.
#[derive(Resource)]
#[resource(model = User, export_query = export_scoped)]
struct NarrowExport;

#[tokio::test]
async fn derived_query_override_scopes_rows() {
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

/// GH #177: `export_query = path` reaches the trait, and the needs the table
/// gathered decide what the narrowed query returns. `NarrowExport` declares
/// no columns, so its export query sees an empty set.
#[tokio::test]
async fn derived_export_query_receives_the_declared_needs() {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    for name in ["Ada", "Bob"] {
        toasty::create!(User {
            name: name.to_string()
        })
        .exec(&mut db)
        .await
        .unwrap();
    }

    let cx = CxTestBuilder::new().app_context(db).build();
    let mut db = argentum_core::db::db(&cx);

    // `query` is still the inherited one (no rows scoped out)...
    let all = NarrowExport::query(&cx).exec(&mut db).await.unwrap();
    assert_eq!(all.len(), 2);

    // ...while the narrowed export query follows the declared includes.
    let none_declared = NarrowExport::export_query(&cx, &IncludeNeeds::new())
        .exec(&mut db)
        .await
        .unwrap();
    assert_eq!(none_declared.len(), 2);

    let declared = NarrowExport::export_query(&cx, &IncludeNeeds::from(["ada"]))
        .exec(&mut db)
        .await
        .unwrap();
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].name, "Ada");
}
