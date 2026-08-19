//! S2 — proves the Toasty + SQLite seam: the workspace's `toasty` dependency
//! (with the `sqlite` driver) can define a model, create rows, query them, and
//! enforce a unique constraint — all against an in-memory database.
//!
//! This is the smallest vertical slice through Toasty. The `db(cx)` helper and
//! `#[memoize]` glue that tie it to Topcoat land with the Db-glue ticket.

use toasty::Db;

#[derive(Debug, toasty::Model)]
struct User {
    #[key]
    #[auto]
    id: uuid::Uuid,

    name: String,

    #[unique]
    email: String,
}

async fn roundtrip_db() -> Db {
    let db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");
    db.push_schema().await.expect("push schema");
    db
}

#[tokio::test]
async fn create_and_query_users() {
    let mut db = roundtrip_db().await;

    toasty::create!(User {
        name: "Alice",
        email: "alice@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create Alice");

    toasty::create!(User {
        name: "Bob",
        email: "bob@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create Bob");

    let everyone: Vec<User> = User::all()
        .order_by(User::fields().name().asc())
        .exec(&mut db)
        .await
        .expect("query all users");

    assert_eq!(everyone.len(), 2);
    assert_eq!(everyone[0].name, "Alice");
    assert_eq!(everyone[1].email, "bob@example.com");

    // `#[unique]` is enforced by the driver, not the application layer.
    let duplicate = toasty::create!(User {
        name: "Imposter",
        email: "alice@example.com",
    })
    .exec(&mut db)
    .await;
    assert!(
        duplicate.is_err(),
        "a duplicate email must be rejected by the unique constraint"
    );
}
