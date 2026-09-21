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

/// GH #189 item 2, measured rather than assumed: is there a **collation
/// disagreement** between the app-side unique check and this database?
///
/// The check probes with the Toasty `eq` filter and skips an edit's unchanged
/// value by comparing trimmed strings — both case-sensitive operations. That
/// only disagrees with the database if the `#[unique]` index compares
/// case-insensitively. Two measurements pin the answer for this configuration:
///
/// 1. The `eq` probe finds the row by its exact stored value, so the probe and
///    the index look at equality the same way.
/// 2. `Alice@example.com` inserts *beside* `alice@example.com`, so the index
///    treats the two as distinct values.
///
/// Together: SQLite's default `BINARY` collation is case- and accent-sensitive,
/// which is exactly what the panel promises — the constraint can reject a
/// duplicate the probe missed only if the column is declared with a
/// non-`BINARY` collation, and no Argentum declaration (or Toasty field
/// attribute) sets one. The test is the record of that finding, so a future
/// driver change that flips the default fails here instead of in production, a
/// second empty submit at a time (GH #189 item 1).
#[tokio::test]
async fn unique_collation_matches_the_app_side_probe() {
    let mut db = roundtrip_db().await;

    toasty::create!(User {
        name: "Alice",
        email: "alice@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create Alice");

    // 1. The probe's `eq` finds the stored value exactly: the same comparison
    //    the index makes, so a duplicate the probe reports is one the index
    //    would refuse too.
    let exact = User::all()
        .filter(User::fields().email().eq("alice@example.com"))
        .exec(&mut db)
        .await
        .expect("probe by exact value");
    assert_eq!(exact.len(), 1, "the probe must find the stored value");

    // 2. A case variant is a different value for the index as well: it stores
    //    beside Alice, so the probe not flagging it is not a missed duplicate.
    toasty::create!(User {
        name: "Alice (cased)",
        email: "Alice@example.com",
    })
    .exec(&mut db)
    .await
    .expect("a case variant is a distinct value under the default collation");

    let cased_probe = User::all()
        .filter(User::fields().email().eq("ALICE@EXAMPLE.COM"))
        .exec(&mut db)
        .await
        .expect("probe by upper-cased value");
    assert!(
        cased_probe.is_empty(),
        "the index and the probe agree that case variants are distinct — \
         a non-BINARY column collation would break this agreement"
    );
}
