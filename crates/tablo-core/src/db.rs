//! The `Db` glue: how Tablo code reaches the pooled Toasty database.
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
/// Pool discipline: while a framework transaction holds its
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
/// to an opaque 500: the driver/SQL text is logged for operators
/// but never reaches the error page. The streamed list already holds this
/// contract through its generic `ErrorState` (logged once at
/// `table_error_view`); mutation/export paths must match it.
///
/// Only infra failures come here. App-hook errors (`create_record` et al.)
/// and explicit guards (404s, 403s, config errors) keep their own mapping;
/// [`hook_failure`] is the seam that separates the two when both arrive
/// through the same record fn.
pub(crate) fn unavailable(source: impl std::fmt::Display) -> topcoat::Error {
    tracing::error!(error = %source, "database unavailable");
    topcoat::Error::from(std::io::Error::other("database unavailable"))
}

/// Map a failed record hook to the error the response carries.
///
/// A hook (`create_record`, `update_record`, `delete_record`, …) is app code,
/// so its error is one of two things: the app's own — a guard's 404, a config
/// error, the default stub's "not implemented" — or the driver's.
/// keeps the app's mapping (a guard must still answer 404, not a 500), while
/// the driver's is an infra failure and takes [`unavailable`]: its text goes
/// to the log, and the page never echoes it. The error's own type settles
/// which one it is, never its message — Toasty owns [`toasty::Error`], so a
/// downcast decides (the same seam the crate uses for
/// `ContentTooLargeError` and `CursorDecodeError`).
pub(crate) fn hook_failure(error: topcoat::Error) -> topcoat::Error {
    if error.is::<toasty::Error>() {
        unavailable(error)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;
    use crate::test_support::User;

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

    /// GH #229: a record hook that fails at the driver is an infra failure, so
    /// the write surfaces the opaque mapping — the property
    /// `unavailable_maps_infra_failures_to_an_opaque_error` pins, reached
    /// through the hook seam.
    #[test]
    fn hook_failure_maps_driver_errors_to_an_opaque_error() {
        let err = super::hook_failure(
            toasty::Error::from_args(format_args!("secret driver gunk: no such table")).into(),
        );
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

    /// GH #229 must not undo GH #174: an app-authored hook error is not the
    /// driver's, so it keeps its own mapping — a guard's 404 stays a 404
    /// rather than becoming the opaque 500.
    #[test]
    fn hook_failure_keeps_an_app_error_intact() {
        let guard: topcoat::Error = topcoat::router::error::not_found().into();
        assert!(
            super::hook_failure(guard).is::<topcoat::router::error::NotFoundError>(),
            "an app-hook error must keep its own mapping"
        );
    }
}
