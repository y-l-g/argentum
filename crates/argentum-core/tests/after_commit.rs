//! The post-commit seam, end to end (GH #112): a side effect that must not
//! survive a rollback runs once per committed write, never when the write did
//! not land, and cannot undo a write that did.
//!
//! Every test here goes through a real `Panel` and `Router`, because the
//! guarantee being pinned is about *where* in the handler the hook runs — after
//! `tx.commit()`, before the response — and no unit test of the hook alone
//! could show that.

use std::collections::HashMap;

use argentum_core::{Committed, Mutation, Resource, Schema, Table, TextColumn, TextInput};
use http::header::LOCATION;
use toasty::Db;
use topcoat::{context::Cx, router::Body};
use uuid::Uuid;

use crate::common::{memory_db, post_fields, router};

#[derive(Debug, toasty::Model, Clone)]
struct Note {
    #[key]
    #[auto]
    id: Uuid,
    title: String,
}

/// What a hook writes: one row per *call*, so "exactly once per write" is
/// countable rather than inferred. `title` records the snapshot the hook was
/// handed, which is how the documented pre-write contract for updates is
/// pinned.
#[derive(Debug, toasty::Model, Clone)]
struct Audit {
    #[key]
    #[auto]
    id: Uuid,
    mutation: String,
    row_key: String,
    title: String,
    rows: i64,
}

/// The audit row's spelling for a mutation — the app's own vocabulary, which
/// is the point of the hook: `Mutation` names the kind, the app spells it.
fn mutation_name(mutation: Mutation) -> &'static str {
    match mutation {
        Mutation::Create => "create",
        Mutation::Update => "update",
        Mutation::Delete => "delete",
    }
}

/// The one line every hook in this file shares: record what it was handed.
async fn audit(cx: &Cx, committed: &Committed<Note>) -> topcoat::Result<()> {
    let first = committed.records().first();
    let mut db = argentum_core::db::db(cx);
    toasty::create!(Audit {
        mutation: mutation_name(committed.mutation()).to_string(),
        row_key: first.map(|note| note.id.to_string()).unwrap_or_default(),
        title: first.map(|note| note.title.clone()).unwrap_or_default(),
        rows: committed.records().len() as i64,
    })
    .exec(&mut db)
    .await
    .map_err(|error| -> topcoat::Error { error.into() })?;
    Ok(())
}

/// A resource that audits everything it commits.
struct AuditedResource;

impl Resource for AuditedResource {
    type Model = Note;

    fn slug() -> String {
        "notes".to_string()
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn can_view(_cx: &Cx, _record: &Note) -> bool {
        true
    }

    fn can_create(_cx: &Cx) -> bool {
        true
    }

    fn can_update(_cx: &Cx, _record: &Note) -> bool {
        true
    }

    fn can_delete(_cx: &Cx, _record: &Note) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Note> {
        Table::r#for(cx)
            .id(|note: &Note| note.id.to_string())
            .pk(|note: &Note| note.id.to_string())
            .paginate(25)
            .columns(TextColumn::r#for(Note::fields().title(), |note: &Note| {
                note.title.clone()
            }))
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new(TextInput::r#for(Note::fields().title()))
    }

    fn hydrate_form_values(_cx: &Cx, record: &Note) -> HashMap<String, String> {
        HashMap::from([("title".to_string(), record.title.clone())])
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Note> {
        toasty::create!(Note {
            title: values.get("title").cloned().unwrap_or_default(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })
    }

    async fn update_record(
        _cx: &Cx,
        mut record: Note,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Note> {
        if let Some(title) = values.get("title") {
            record.title = title.clone();
        }
        toasty::update!(record {
            title: record.title.clone(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })?;
        // The instance update reloads the row, so this is the committed state
        // the hook must see (GH #112).
        Ok(record)
    }

    async fn delete_record(
        _cx: &Cx,
        record: Note,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<()> {
        Note::filter(Note::fields().id().eq(record.id))
            .delete()
            .exec(&mut *ex)
            .await
            .map_err(|error| -> topcoat::Error { error.into() })?;
        Ok(())
    }

    async fn bulk_delete_records(
        _cx: &Cx,
        records: Vec<Note>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<()> {
        for record in records {
            Note::filter(Note::fields().id().eq(record.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|error| -> topcoat::Error { error.into() })?;
        }
        Ok(())
    }

    async fn after_commit(cx: &Cx, committed: Committed<Note>) -> topcoat::Result<()> {
        audit(cx, &committed).await
    }
}

/// A resource that declares no hook: the default is a no-op, so nothing about
/// its writes changes (GH #112's "existing resources are unaffected").
struct PlainResource;

impl Resource for PlainResource {
    type Model = Note;

    fn slug() -> String {
        "plain-notes".to_string()
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn can_create(_cx: &Cx) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Note> {
        Table::r#for(cx)
            .id(|note: &Note| note.id.to_string())
            .columns(TextColumn::r#for(Note::fields().title(), |note: &Note| {
                note.title.clone()
            }))
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new(TextInput::r#for(Note::fields().title()))
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Note> {
        toasty::create!(Note {
            title: values.get("title").cloned().unwrap_or_default(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })
    }
}

/// A resource whose record fn fails: nothing commits, so the hook must not run.
struct FailingWriteResource;

impl Resource for FailingWriteResource {
    type Model = Note;

    fn slug() -> String {
        "failing-writes".to_string()
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn can_create(_cx: &Cx) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Note> {
        Table::r#for(cx)
            .id(|note: &Note| note.id.to_string())
            .columns(TextColumn::r#for(Note::fields().title(), |note: &Note| {
                note.title.clone()
            }))
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new(TextInput::r#for(Note::fields().title()))
    }

    async fn create_record(
        _cx: &Cx,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Note> {
        Err(std::io::Error::other("the write refused itself").into())
    }

    async fn after_commit(cx: &Cx, committed: Committed<Note>) -> topcoat::Result<()> {
        audit(cx, &committed).await
    }
}

/// A resource whose hook fails: the write is already committed, so the failure
/// is logged and the write stands (GH #112's "failures handled loudly without
/// rolling back the write").
struct FailingHookResource;

impl Resource for FailingHookResource {
    type Model = Note;

    fn slug() -> String {
        "failing-hooks".to_string()
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn can_create(_cx: &Cx) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Note> {
        Table::r#for(cx)
            .id(|note: &Note| note.id.to_string())
            .columns(TextColumn::r#for(Note::fields().title(), |note: &Note| {
                note.title.clone()
            }))
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new(TextInput::r#for(Note::fields().title()))
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> topcoat::Result<Note> {
        toasty::create!(Note {
            title: values.get("title").cloned().unwrap_or_default(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|error| -> topcoat::Error { error.into() })
    }

    async fn after_commit(cx: &Cx, committed: Committed<Note>) -> topcoat::Result<()> {
        // Ran, and left its evidence, before failing.
        audit(cx, &committed).await?;
        Err(std::io::Error::other("the webhook is down").into())
    }
}

async fn seeded_db() -> Db {
    memory_db(toasty::models!(Note, Audit)).await
}

async fn seed_note(db: &Db, title: &str) -> Note {
    let mut db = db.clone();
    toasty::create!(Note {
        title: title.to_string(),
    })
    .exec(&mut db)
    .await
    .expect("seed note")
}

async fn notes(db: &Db) -> Vec<Note> {
    let mut db = db.clone();
    Note::all().exec(&mut db).await.expect("query notes")
}

async fn audits(db: &Db) -> Vec<Audit> {
    let mut db = db.clone();
    Audit::all().exec(&mut db).await.expect("query audits")
}

#[tokio::test]
async fn a_create_audits_the_row_it_committed_exactly_once() {
    let db = seeded_db().await;
    let router = router::<AuditedResource>(db.clone());

    let response = post_fields(&router, "/admin/notes/create", &[("title", "Alpha")]).await;
    assert_eq!(response.status(), 303, "a valid create redirects");

    let created = notes(&db).await;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].title, "Alpha");

    // One write, one hook call — and the call names the row the create
    // returned, which is the only place the framework can learn the key.
    let audits = audits(&db).await;
    assert_eq!(audits.len(), 1, "one commit is one hook call");
    assert_eq!(audits[0].mutation, "create");
    assert_eq!(audits[0].row_key, created[0].id.to_string());
    assert_eq!(audits[0].rows, 1);
}

#[tokio::test]
async fn an_edit_and_a_delete_each_audit_the_row_they_named() {
    let db = seeded_db().await;
    let router = router::<AuditedResource>(db.clone());
    let note = seed_note(&db, "Alpha").await;

    let response = post_fields(
        &router,
        &format!("/admin/notes/{}/edit", note.id),
        &[("title", "Beta")],
    )
    .await;
    assert_eq!(response.status(), 303, "a valid edit redirects");
    assert_eq!(notes(&db).await[0].title, "Beta");

    // The hook is handed the row the handler loaded inside the transaction, so
    // its title is the pre-write one — documented, and pinned here.
    let after_edit = audits(&db).await;
    assert_eq!(after_edit.len(), 1);
    assert_eq!(after_edit[0].mutation, "update");
    assert_eq!(after_edit[0].row_key, note.id.to_string());
    assert_eq!(
        after_edit[0].title, "Beta",
        "an update hands over the committed row, not the loaded snapshot"
    );

    let response = post_fields(
        &router,
        &format!("/admin/notes/{}/delete", note.id),
        &[("confirm", "1")],
    )
    .await;
    assert_eq!(response.status(), 303, "a confirmed delete redirects");
    assert!(notes(&db).await.is_empty());

    let after_delete = audits(&db).await;
    assert_eq!(after_delete.len(), 2, "two writes, two calls");
    assert_eq!(after_delete[1].mutation, "delete");
    assert_eq!(after_delete[1].row_key, note.id.to_string());
}

#[tokio::test]
async fn a_bulk_delete_is_one_call_for_the_whole_batch() {
    let db = seeded_db().await;
    let router = router::<AuditedResource>(db.clone());
    let first = seed_note(&db, "Alpha").await;
    let second = seed_note(&db, "Beta").await;

    let ids = format!("{},{}", first.id, second.id);
    let response = post_fields(
        &router,
        "/admin/notes/bulk-delete",
        &[("ids", &ids), ("confirm", "1")],
    )
    .await;
    assert_eq!(response.status(), 303);
    assert!(notes(&db).await.is_empty());

    let audits = audits(&db).await;
    assert_eq!(
        audits.len(),
        1,
        "one bulk write is one hook call, not one per row"
    );
    assert_eq!(audits[0].mutation, "delete");
    assert_eq!(audits[0].rows, 2, "both rows travel in the one call");
}

#[tokio::test]
async fn a_refused_submit_never_reaches_the_hook() {
    let db = seeded_db().await;
    let router = router::<AuditedResource>(db.clone());

    // `title` is required: validation re-renders the form and the transaction
    // is never opened.
    let response = post_fields(&router, "/admin/notes/create", &[("title", "")]).await;
    assert_eq!(response.status(), 200, "the form re-renders");

    assert!(notes(&db).await.is_empty());
    assert!(
        audits(&db).await.is_empty(),
        "a rollback must not produce the side effect"
    );
}

#[tokio::test]
async fn a_failed_write_never_reaches_the_hook() {
    let db = seeded_db().await;
    let router = router::<FailingWriteResource>(db.clone());

    let response = post_fields(
        &router,
        "/admin/failing-writes/create",
        &[("title", "Alpha")],
    )
    .await;
    assert!(
        response.status().is_server_error(),
        "a record fn that refuses is an error, got {}",
        response.status()
    );

    assert!(notes(&db).await.is_empty());
    assert!(
        audits(&db).await.is_empty(),
        "the transaction rolled back, so nothing committed"
    );
}

#[tokio::test]
async fn a_failing_hook_does_not_undo_the_write() {
    let db = seeded_db().await;
    let router = router::<FailingHookResource>(db.clone());

    let response = post_fields(
        &router,
        "/admin/failing-hooks/create",
        &[("title", "Alpha")],
    )
    .await;
    assert_eq!(
        response.status(),
        303,
        "the write is committed, so the response reports it"
    );

    assert_eq!(
        notes(&db).await.len(),
        1,
        "a failed side effect cannot roll back a committed write"
    );
    assert_eq!(
        audits(&db).await.len(),
        1,
        "the hook ran; only its failure is swallowed"
    );
}

#[tokio::test]
async fn a_resource_without_the_hook_writes_exactly_as_before() {
    let db = seeded_db().await;
    let router = router::<PlainResource>(db.clone());

    let response = post_fields(&router, "/admin/plain-notes/create", &[("title", "Alpha")]).await;
    assert_eq!(response.status(), 303, "the default hook is a no-op");
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/plain-notes")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert!(response.status().is_success(), "the list still renders");
    assert!(
        response.headers().get(LOCATION).is_none(),
        "a plain GET is not a redirect"
    );

    assert_eq!(notes(&db).await.len(), 1);
    // No `audits` assertion here: this resource declares no hook, so nothing in
    // the framework could write that table and the check could never fail
    // (GH #216). The hook-bearing tests above own it.
}
