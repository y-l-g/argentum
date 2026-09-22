//! What a mutation committed, and the one place the framework says so
//! (GH #112).
//!
//! The write handlers own the transaction (GH #84): a `Resource` record fn
//! writes through `&mut dyn toasty::Executor` and the framework commits. That
//! leaves nowhere correct for a side effect that must *not* happen on a
//! rollback — an email, a webhook, an audit row, cache invalidation: doing it
//! inside the record fn leaks it when the transaction rolls back, and opening a
//! second handle while the transaction holds the pool is the discipline
//! problem the handlers exist to avoid.
//!
//! So the framework reports the commit instead: [`Committed`] names the
//! mutation and the rows it wrote, [`Resource::after_commit`] receives it once
//! per successful write, and the transaction is gone by then.

use topcoat::context::Cx;

use super::Resource;

/// The kind of mutation a record fn performed.
///
/// The vocabulary `Action` names in `CONTEXT.md`, as a value: the framework
/// knows which of the four record fns ran, so an audit row or a webhook
/// payload does not have to be spelled per call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutation {
    Create,
    Update,
    Delete,
}

/// What one committed mutation wrote, handed to
/// [`Resource::after_commit`](super::Resource::after_commit).
///
/// The handlers build one on every successful write: `created` carries the row
/// a create returned, `updated` the row an update returned (the committed
/// state), `deleted` the rows a delete or bulk delete removed — one value per
/// write, so a bulk delete is *one* `Committed` however many rows it took. The
/// constructors are public for the other direction: an app exercising its own
/// `after_commit` in a test builds the value it wants to hand it.
#[derive(Debug, Clone)]
pub struct Committed<M> {
    mutation: Mutation,
    records: Vec<M>,
}

impl<M> Committed<M> {
    /// The row a create wrote.
    pub fn created(record: M) -> Self {
        Self {
            mutation: Mutation::Create,
            records: vec![record],
        }
    }

    /// The row an update wrote, as it stands after the write — what
    /// `update_record` returned, not the snapshot the handler loaded.
    pub fn updated(record: M) -> Self {
        Self {
            mutation: Mutation::Update,
            records: vec![record],
        }
    }

    /// The rows a delete removed, as they were before the delete.
    pub fn deleted(records: Vec<M>) -> Self {
        Self {
            mutation: Mutation::Delete,
            records,
        }
    }

    /// Which record fn ran.
    pub fn mutation(&self) -> Mutation {
        self.mutation
    }

    /// The rows the mutation wrote, in the order the handler had them.
    pub fn records(&self) -> &[M] {
        &self.records
    }
}

/// Deliver a committed mutation to the app (GH #112).
///
/// The framework's single call site, so the failure policy cannot drift
/// between the four write handlers: a hook that returns `Err` is **logged and
/// ignored**. The write is committed — the row is in the database, the
/// response is the redirect the user earned — so turning a failed email into
/// an error page would misreport what happened, and rolling back is not
/// available. Retries and delivery guarantees are deliberately not the
/// framework's promise; an app that needs them writes its own outbox here.
///
/// A hook that *panics* is not caught here: Topcoat isolates a panicking
/// request into a 500, which is loud and still leaves the write committed. That
/// is a bug in the hook, not a reported failure, and it is why this only
/// handles `Err`.
pub(crate) async fn run_after_commit<R: Resource>(cx: &Cx, committed: Committed<R::Model>) {
    if let Err(error) = R::after_commit(cx, committed).await {
        tracing::error!(
            error = %error,
            resource = R::slug(),
            "after_commit failed; the write stays committed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Row {
        id: u32,
    }

    #[test]
    fn committed_names_the_mutation_and_its_rows() {
        let created = Committed::created(Row { id: 1 });
        assert_eq!(created.mutation(), Mutation::Create);
        assert_eq!(created.records(), [Row { id: 1 }]);

        let updated = Committed::updated(Row { id: 2 });
        assert_eq!(updated.mutation(), Mutation::Update);

        // A bulk delete is one value however many rows it took.
        let deleted = Committed::deleted(vec![Row { id: 3 }, Row { id: 4 }]);
        assert_eq!(deleted.mutation(), Mutation::Delete);
        assert_eq!(
            deleted.records(),
            [Row { id: 3 }, Row { id: 4 }],
            "the rows keep the order the handler had them"
        );
    }
}
