//! The `Db` glue: how Argentum code reaches the pooled Toasty database.
//!
//! `Db` is registered once on the app context (`app_context::<Db>`), and each
//! request clones it — a cheap `Arc` bump — before running Toasty statements,
//! which require `&mut Db`.

use toasty::Db;
use topcoat::context::{Cx, app_context};

/// Returns the pooled [`Db`] registered on the app context.
///
/// Cloning the `Db` is cheap; Toasty statements need `&mut Db`, so callers do
/// `let mut db = db(cx); ...exec(&mut db).await?`.
///
/// Pool discipline (GH #84): while a framework transaction holds its
/// connection, no other handle may run statements — with a single-connection
/// pool (notably `sqlite::memory:`) a second handle blocks forever. Mutation
/// handlers therefore keep the tx strictly around check + write + commit and
/// `drop(tx)` before any re-render (form option loaders open their own
/// handle).
#[inline]
pub fn db(cx: &Cx) -> Db {
    app_context::<Db>(cx).clone()
}

/// Map a database infrastructure failure (pool/tx open, probe/exec, commit)
/// to an opaque 500 (GH #174): the driver/SQL text is logged for operators
/// but never reaches the error page. The streamed list already holds this
/// contract through its generic `ErrorState` (logged once at
/// `grid_error_view`); mutation/export paths must match it.
///
/// Only infra failures come here. App-hook errors (`create_record` et al.)
/// and explicit guards (404s, 403s, config errors) keep their own mapping.
pub(crate) fn unavailable(source: impl std::fmt::Display) -> topcoat::Error {
    tracing::error!(error = %source, "database unavailable");
    topcoat::Error::from(std::io::Error::other("database unavailable"))
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    #[derive(Debug, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    async fn seeded_db() -> Db {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");
        db.push_schema().await.expect("push schema");

        toasty::create!(User { name: "Ada" })
            .exec(&mut db)
            .await
            .expect("seed user");
        db
    }

    #[tokio::test]
    async fn db_returns_the_app_context_db() {
        let seeded = seeded_db().await;
        let cx = CxTestBuilder::new().app_context(seeded.clone()).build();

        let mut from_helper = db(&cx);

        // The helper must return the same pooled `Db`, so a query through it
        // sees the rows seeded through the original handle.
        let users: Vec<User> = User::all()
            .exec(&mut from_helper)
            .await
            .expect("query users");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "Ada");
    }

    #[test]
    fn unavailable_maps_infra_failures_to_an_opaque_error() {
        // GH #174: driver text is for the logs, never the error page.
        let err = super::unavailable("secret driver gunk: no such table");
        let rendered = err.to_string();
        assert!(
            rendered.contains("database unavailable"),
            "the opaque message must survive, got {rendered}"
        );
        assert!(
            !rendered.contains("gunk"),
            "driver text must not leak, got {rendered}"
        );
    }
}
