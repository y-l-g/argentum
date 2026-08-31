use argentum_core::Resource;
use toasty::Db;
use topcoat::context::{Cx, CxTestBuilder};

fn scoped(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
    toasty::stmt::Query::<toasty::stmt::List<User>>::all().filter(User::fields().name().eq("Ada"))
}

#[derive(Debug, toasty::Model)]
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
