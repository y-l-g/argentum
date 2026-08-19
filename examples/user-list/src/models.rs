//! The example's Toasty model, the request-memoized loader, and a seed.

use std::sync::atomic::{AtomicUsize, Ordering};

use toasty::Db;
use topcoat::context::{Cx, memoize};

pub use argentum_core::db::db;

/// A user record shown in the admin list page.
#[derive(Debug, toasty::Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: uuid::Uuid,

    pub name: String,

    #[unique]
    pub email: String,
}

/// Number of times the loader body has run for the current request. Only
/// asserted by the memoize-dedup test below; no other test in this crate
/// touches `query_users`, so the count is deterministic.
static LOADER_RUNS: AtomicUsize = AtomicUsize::new(0);

/// Loads all users, at most once per request.
///
/// `#[memoize]` caches the result for the duration of a request, so a page
/// that lists users (and any other view sharing the query) issues the Toasty
/// query once, and concurrent callers share one in-flight future.
#[memoize(as_ref)]
async fn query_users(cx: &Cx) -> topcoat::Result<Vec<User>> {
    LOADER_RUNS.fetch_add(1, Ordering::SeqCst);
    User::all()
        .order_by(User::fields().name().asc())
        .exec(&mut db(cx))
        .await
        .map_err(Into::into)
}

/// Returns the request-memoized list of users.
///
/// `#[memoize(as_ref)]` hands back `Result<&Vec<User>, &Error>`; the error is
/// re-wrapped into an owned `Error` because `topcoat::Error` (anyhow-backed)
/// is not `Clone`. This mirrors the canonical pattern in
/// `tokio-rs/topcoat/demos/coffee-shop/src/models.rs`.
pub async fn users(cx: &Cx) -> topcoat::Result<&Vec<User>> {
    query_users(cx)
        .await
        .map_err(|error| std::io::Error::other(error.to_string()).into())
}

/// Seeds the database with a few users (startup path for the example).
pub async fn seed(db: &mut Db) -> toasty::Result<()> {
    toasty::create!(User::[
        { name: "Ada Lovelace", email: "ada@example.com" },
        { name: "Grace Hopper", email: "grace@example.com" },
        { name: "Linus Torvalds", email: "linus@example.com" },
    ])
    .exec(db)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use topcoat::context::CxTestBuilder;

    use super::*;

    async fn seeded_db() -> Db {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");
        db.push_schema().await.expect("push schema");
        seed(&mut db).await.expect("seed");
        db
    }

    /// The loader is `#[memoize]`d, so concurrent callers share one in-flight
    /// future and the body runs exactly once.
    #[tokio::test]
    async fn loader_dedups_concurrent_calls() {
        let cx = Arc::new(CxTestBuilder::new().app_context(seeded_db().await).build());

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let cx = Arc::clone(&cx);
            tasks.push(tokio::spawn(async move {
                users(&cx).await.expect("memoized load").len()
            }));
        }

        let mut lengths = Vec::new();
        for task in tasks {
            lengths.push(task.await.expect("task"));
        }
        assert!(
            lengths.iter().all(|&len| len == 3),
            "all callers must see 3 users, got {lengths:?}"
        );

        assert_eq!(
            LOADER_RUNS.load(Ordering::SeqCst),
            1,
            "the memoized loader body must run once across concurrent callers"
        );
    }
}
