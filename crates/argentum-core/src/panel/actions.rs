//! Delete / bulk-delete / CSV export handlers plus their caps and parsers.
//!
//! Fetch, policy checks, and writes share one framework transaction:
//! a mid-loop failure deletes zero rows.

use topcoat::{
    Result,
    context::Cx,
    router::{
        Body, RouteFuture,
        error::{forbidden, see_other},
    },
    view::{BoxView, HoistView, internal::ThenView},
};

use super::{
    forms::{parse_form_body, truthy},
    gate, list_url,
};
use crate::{
    db::db,
    notification::{Notification, notify_write_failure, set_notification},
    resource::Committed,
};

/// Failure-toast wording for the delete handlers.
const WRITE_DELETE: &str = "delete the record";
const WRITE_BULK_DELETE: &str = "delete the selected rows";
use crate::{
    resource::{OrderMode, Resource, Table, TableState, clamp_query_term},
    schema::OptionLoadError,
};

/// Fetch one record by its URL `id` through the tenancy-scoped query seam.
///
/// The string id is parsed against the model's primary-key type and the PK
/// filter is ANDed onto the tenant-scoped
/// [`scoped_query`](crate::resource::scoped_query) (ADR-0002), so
/// tenancy and soft-delete scoping both hold. Fetches the one row by key
/// instead of loading every row and matching keys in memory — O(N) rows per
/// edit/delete, leaking the whole table before the policy check.
///
/// A malformed or unknown id maps to 404, not a query error.
///
/// Runs on the caller's executor: mutation handlers pass the open framework
/// transaction so the fetched snapshot is the checked snapshot.
pub(crate) async fn find_by_key<R: Resource>(
    cx: &Cx,
    id: &str,
    ex: &mut dyn toasty::Executor,
) -> Result<R::Model> {
    find_by_key_in::<R>(id, ex, || crate::resource::scoped_query::<R>(cx)).await
}

/// [`find_by_key`] for a loader that reads only the record's own columns
/// the same PK filter over
/// [`scoped_query_with`](crate::resource::scoped_query_with) with an empty
/// [`IncludeNeeds`](crate::resource::IncludeNeeds), so the edit handler and
/// delete do not load the relations the record's list or detail page reads. A
/// resource that overrides nothing keeps its full
/// [`query`](crate::resource::Resource::query).
pub(crate) async fn find_by_key_narrowed<R: Resource>(
    cx: &Cx,
    id: &str,
    ex: &mut dyn toasty::Executor,
) -> Result<R::Model> {
    find_by_key_in::<R>(id, ex, || {
        crate::resource::scoped_query_with::<R>(cx, &crate::resource::IncludeNeeds::default())
    })
    .await
}

/// The error a resource with a composite primary key reports when the URL or
/// batch carries no single-key representation. `None` means the model
/// has a single-column key.
fn composite_pk_error<R: Resource>() -> Option<topcoat::Error> {
    if !crate::schema::pk_is_composite::<R::Model>() {
        return None;
    }
    tracing::error!(
        resource = R::slug(),
        "composite primary key has no URL representation"
    );
    Some(topcoat::Error::from(std::io::Error::other(format!(
        "resource '{}' has a composite primary key, which has no URL representation (GH #95)",
        R::slug()
    ))))
}

/// The shared body of [`find_by_key`] and [`find_by_key_narrowed`]: parse the
/// URL id against the model's primary key, then fetch the one row through
/// `seed`.
///
/// `seed` is a closure so the composite-PK misdeclaration is reported before
/// the scoped query is built.
async fn find_by_key_in<R: Resource>(
    id: &str,
    ex: &mut dyn toasty::Executor,
    seed: impl FnOnce() -> Result<toasty::stmt::Query<toasty::stmt::List<R::Model>>>,
) -> Result<R::Model> {
    let Some(expr) = crate::schema::pk_eq_expr::<R::Model>(id) else {
        // Composite PKs have no URL representation: fail loudly so
        // the misconfiguration surfaces instead of 404ing every id.
        if let Some(error) = composite_pk_error::<R>() {
            return Err(error);
        }
        return Err(topcoat::router::error::not_found().into());
    };
    seed()?
        .filter(expr)
        .first()
        .exec(&mut *ex)
        .await
        .map_err(crate::db::unavailable)?
        .ok_or_else(topcoat::router::error::not_found)
        .map_err(Into::into)
}

/// Load the record the request names, scoped and policy-checked.
///
/// Reads the `{id}` path param, loads through the tenant-scoped query (which
/// turns an unknown *or* out-of-scope id into one 404), and returns 403 unless
/// `can_view` accepts the loaded snapshot.
///
/// Callers run [`gate`](super::gate) first, add their own policy on top
/// (`can_update` for the edit page), and 404 a page the resource does not
/// declare (`R::viewed` for the view page).
pub(crate) async fn load_viewable<R: Resource>(
    cx: &Cx,
    ex: &mut dyn toasty::Executor,
) -> Result<R::Model> {
    load_viewable_in::<R>(cx, ex, false).await
}

/// [`load_viewable`] for a loader that reads only the record's own columns
/// the edit page hydrates its fields from the record, so it does
/// not load the relations the record's list or detail page reads.
pub(crate) async fn load_viewable_narrowed<R: Resource>(
    cx: &Cx,
    ex: &mut dyn toasty::Executor,
) -> Result<R::Model> {
    load_viewable_in::<R>(cx, ex, true).await
}

/// The shared body of [`load_viewable`] and [`load_viewable_narrowed`]: read
/// the `{id}` path param, load through the tenant-scoped query, then 403
/// unless `can_view` accepts the snapshot.
///
/// `narrowed` selects the loader that reads only the record's own columns.
async fn load_viewable_in<R: Resource>(
    cx: &Cx,
    ex: &mut dyn toasty::Executor,
    narrowed: bool,
) -> Result<R::Model> {
    let id = topcoat::router::path_param_segment(cx, "id").to_string();
    let record = if narrowed {
        find_by_key_narrowed::<R>(cx, &id, ex).await?
    } else {
        find_by_key::<R>(cx, &id, ex).await?
    };
    if !R::can_view(cx, &record) {
        return Err(topcoat::router::error::forbidden().into());
    }
    Ok(record)
}

/// Delete action POST — confirmation-marked, policy-checked, and run in the
/// framework transaction: the checked record flows into the write.
///
/// The confirmation is the row's alert dialog on the list page: the
/// Delete link opens `?delete=<key>` and the dialog's form POSTs here with
/// `confirm=1`. Authentication comes before any DB work: the CSRF
/// check and the confirmation marker run first, so a forged POST answers 403
/// without opening a transaction, holding a pooled connection across the body
/// read, or probing record existence (create/bulk-delete ordering).
/// The dialog itself is deliberately fetch-free and policy-blind: it carries
/// no record data and embeds only the caller's own CSRF token, and the
/// policy/tenancy checks run against the loaded record here.
pub(crate) fn resource_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::<_, BoxView<'_>>::new(
        async move {
            gate::<R>(cx)?;
            // Delete/bulk-delete carry no file parts: only the values half is read.
            let values = parse_form_body(cx, body).await?.values;
            crate::csrf::verify(cx, &values)?;
            let confirmed = values.get("confirm").is_some_and(|v| truthy(v));
            if !confirmed {
                // The confirmation UI is the list-page alert dialog:
                // the row link opens `?delete=<key>` and the dialog's form carries
                // `confirm=1`. This route only accepts that confirmed POST, so a
                // missing marker is a malformed client, not a user path.
                return Err(
                    topcoat::router::error::bad_request("delete requires confirmation").into(),
                );
            }
            // Confirmed and authenticated: open the transaction only now (GH
            // #144), fetch through the tenant-scoped query, check Policy against the
            // loaded record, and delete inside the tx — commit makes the checked
            // delete durable, any error rolls it back. Delete takes
            // the edit contract: `can_view` plus
            // `can_delete` — a record that cannot be viewed cannot be deleted
            // by UUID-guessing the route.
            let mut db = db(cx);
            let mut tx = db.transaction().await.map_err(crate::db::unavailable)?;
            let id = topcoat::router::path_param_segment(cx, "id").to_string();
            // The delete path reads only the record's own columns:
            // `can_view`/`can_delete` are Rust predicates over those, and
            // `delete_record` consumes the snapshot.
            let record = find_by_key_narrowed::<R>(cx, &id, &mut tx).await?;
            if !R::can_view(cx, &record) {
                return Err(forbidden().into());
            }
            if !R::can_delete(cx, &record) {
                return Err(forbidden().into());
            }
            // `delete_record` consumes the record, and the hook names what was
            // removed: the pre-delete snapshot, since the row is gone
            // by the time it runs.
            let committed_record = record.clone();
            if let Err(error) = R::delete_record(cx, record, &mut tx).await {
                notify_write_failure(cx, WRITE_DELETE);
                // Same seam as create/update: the driver's text
                // stays in the log, an app-authored hook error keeps its own.
                return Err(crate::db::hook_failure(error));
            }
            if let Err(error) = tx.commit().await {
                notify_write_failure(cx, WRITE_DELETE);
                return Err(crate::db::unavailable(error));
            }
            // Post-commit: the tx is gone, so the hook may open its
            // own handle, and a rollback above never reaches this line.
            crate::resource::run_after_commit::<R>(cx, Committed::deleted(vec![committed_record]))
                .await;
            set_notification(cx, Notification::success("Deleted"));
            Err(see_other(list_url(cx, &R::slug())).into())
        },
    )))
}

/// Bulk delete POST — ids via `ids` form field (comma-separated).
///
/// Identity is the typed PK fetch alone: the display closure
/// `Table::id` is never re-matched, so non-canonical keys (uppercase UUID,
/// email key) cannot 404 a batch whose rows exist. Bounded by
/// `MAX_BULK_IDS` so the `IN` list cannot be amplified into a DoS.
/// Fetch, policy checks, and deletes share one framework transaction
/// a mid-loop failure deletes zero rows.
pub(crate) fn resource_bulk_delete<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::<_, BoxView<'_>>::new(
        async move {
            gate::<R>(cx)?;
            // Delete/bulk-delete carry no file parts: only the values half is read.
            let values = parse_form_body(cx, body).await?.values;
            crate::csrf::verify(cx, &values)?;
            // Confirmation marker, mirroring the row delete: the bulk
            // bar's dialog carries `confirm=1`, so a POST without it did not
            // come from the confirming control. Checked after CSRF verification
            // and before any DB work — a forged POST answers 400
            // without touching a connection.
            if !values.get("confirm").is_some_and(|v| truthy(v)) {
                return Err(topcoat::router::error::bad_request(
                    "bulk delete requires confirmation",
                )
                .into());
            }
            let ids_raw = values.get("ids").cloned().unwrap_or_default();
            let ids = parse_bulk_ids(&ids_raw, MAX_BULK_IDS);
            if ids.is_empty() {
                // No ids is a validation miss, not a raw 400 page:
                // the bulk bar cannot submit without a selection, so only a
                // crafted (or stale) POST gets here — answer like any other
                // mutation, with the list and the reason.
                set_notification(cx, Notification::error("Select at least one row to delete"));
                return Err(see_other(list_url(cx, &R::slug())).into());
            }
            if ids.len() > MAX_BULK_IDS {
                return Err(topcoat::router::error::bad_request(format!(
                    "too many ids (max {MAX_BULK_IDS})"
                ))
                .into());
            }
            // Fetch only the requested rows through the tenancy-scoped seam:
            // one `pk IN (…)` query replaces the #75 item-1
            // fetch-everything-then-match loop. A malformed id cannot exist and
            // maps to 404; a missing/wrong-tenant id makes the fetch come back
            // short and 404s as well.
            let keys: Vec<&str> = ids.iter().map(String::as_str).collect();
            let Some(pk_filter) = crate::schema::pk_in_expr::<R::Model>(&keys) else {
                if let Some(error) = composite_pk_error::<R>() {
                    return Err(error);
                }
                return Err(topcoat::router::error::not_found().into());
            };
            let mut db = db(cx);
            let mut tx = db.transaction().await.map_err(crate::db::unavailable)?;
            // The batch fetch reads only the records' own columns:
            // the policy predicates and the write below never touch a relation.
            let rows = crate::resource::scoped_query_with::<R>(
                cx,
                &crate::resource::IncludeNeeds::default(),
            )?
            .filter(pk_filter)
            .exec(&mut tx)
            .await
            .map_err(crate::db::unavailable)?;
            if rows.len() != ids.len() {
                return Err(topcoat::router::error::not_found().into());
            }
            for rec in &rows {
                // Edit contract on every row: viewing precedes
                // deleting, same as the edit GET/POST pair.
                if !R::can_view(cx, rec) {
                    return Err(forbidden().into());
                }
                if !R::can_delete(cx, rec) {
                    return Err(forbidden().into());
                }
            }
            // All checks passed — perform bulk delete inside the tx, then
            // commit once. Any error drops `tx` uncommitted: zero rows
            // deleted, never half-applied.
            // The hook names the whole batch: a bulk delete is one
            // write, so it is one `after_commit` call, not one per row. Keeping
            // a copy is the price of that (bounded by `MAX_BULK_IDS`); handing
            // the rows over by reference would mean changing two record-fn
            // signatures for a copy this small.
            let committed_rows = rows.clone();
            if let Err(error) = R::bulk_delete_records(cx, rows, &mut tx).await {
                notify_write_failure(cx, WRITE_BULK_DELETE);
                // Same seam as the row delete.
                return Err(crate::db::hook_failure(error));
            }
            if let Err(error) = tx.commit().await {
                notify_write_failure(cx, WRITE_BULK_DELETE);
                return Err(crate::db::unavailable(error));
            }
            crate::resource::run_after_commit::<R>(cx, Committed::deleted(committed_rows)).await;
            set_notification(cx, Notification::success("Bulk deleted"));
            Err(see_other(list_url(cx, &R::slug())).into())
        },
    )))
}

/// Max ids accepted by bulk delete: bounds the `IN` list.
const MAX_BULK_IDS: usize = 400;

/// Max receivable rows an export will deliver: the chunked walk
/// scans at most `MAX_EXPORT_ROWS + 1` raw rows and anything past the cap is
/// a 413, so a 100k-row table stays bounded instead of buffering `Vec<Model>`
/// plus `String` without end. A full raw window with rows left beyond it is a
/// 413 too, even when fewer rows are viewable, so the export never
/// returns a partial file.
const MAX_EXPORT_ROWS: usize = 10_000;

/// Rows per cursor chunk on the export walk: each phase fetches
/// this many models at a time instead of materializing the whole export
/// window, so a 10k-row export holds one chunk plus one CSV fragment.
const EXPORT_CHUNK_ROWS: usize = 500;

/// Reject an export whose visible row count ran past the cap.
/// Extracted from the handler so the 413 mapping is testable at the boundary
/// without materializing 10k rows in a test database.
fn enforce_export_cap_count(count: usize) -> Result<(), topcoat::Error> {
    if count > MAX_EXPORT_ROWS {
        Err(topcoat::router::error::content_too_large().into())
    } else {
        Ok(())
    }
}

/// The longest slug a `Content-Disposition` filename keeps: the
/// header value stays bounded even for an oversized override.
const MAX_EXPORT_FILENAME_LEN: usize = 100;

/// Sanitize the export's `Content-Disposition` filename: `slug`
/// is an overridable free-form `String`, and Topcoat route validation accepts
/// quote and CR/LF segments, so a hostile override would otherwise split the
/// response header. Quote, backslash, and control characters are dropped and
/// the length is capped before the `.csv` suffix. Non-ASCII overrides pass
/// through as obs-text (browsers render them; an RFC 6266 `filename*` is
/// future work).
fn export_filename(slug: &str) -> String {
    let safe: String = slug
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .take(MAX_EXPORT_FILENAME_LEN)
        .collect();
    if safe.is_empty() {
        "export.csv".to_string()
    } else {
        format!("{safe}.csv")
    }
}

/// `?bom=1` opts into a UTF-8 BOM prefix on the CSV body for Excel.
fn export_wants_bom(cx: &Cx) -> bool {
    let Some(parts) = topcoat::context::try_request_context::<http::request::Parts>(cx) else {
        return false;
    };
    let Some(query) = parts.uri.query() else {
        return false;
    };
    form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == "bom" && v == "1")
}

/// Parse + dedupe bulk `ids` while preserving order, so a repeated id can't
/// make the fetched-rows count check misfire.
///
/// `max` bounds the parse itself, not just the final list: a 10 MiB
/// body of distinct ids stops at `max + 1` entries (which the handler then
/// rejects with 400) instead of allocating millions of strings while the
/// `MAX_BULK_IDS` check waits for the parse to finish. Deduping uses a set, so
/// the scan stays linear in the number of ids.
///
/// Known limit: the split happens after url-decoding, so a
/// `String`-PK id containing a literal comma (`%2C`) splits into phantom
/// ids and the batch 404s. Comma-bearing string PKs need a different
/// transport (future work); all other PK types are comma-free.
fn parse_bulk_ids(raw: &str, max: usize) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if seen.insert(s) {
            ids.push(s.to_string());
            if ids.len() > max {
                break;
            }
        }
    }
    ids
}

/// CSV export — the tenant-scoped `export_query` + `Table` filters/sort,
/// downloads `text/csv`.
///
/// Streams the response as a chunked body: the filtered query is
/// walked in cursor chunks ([`EXPORT_CHUNK_ROWS`] rows at a time) and each
/// chunk's CSV is written incrementally, so a 10k-row export holds one chunk
/// plus one CSV fragment instead of `Vec<Model>` + one joined `String`.
///
/// Two passes keep the pre-body contract. First a bounded visibility scan
/// counts receivable rows inside the `MAX_EXPORT_ROWS + 1` raw window — the
/// 413 reflects what the caller may receive, and a window
/// that fills with rows left beyond it is a 413 too, because those rows may
/// be viewable and dropping them would ship a partial file. Both
/// refusals happen before any byte is sent. The scan renders no cell, so it
/// asks for no relation includes. Then the streaming pass re-walks
/// the same window and emits header + rows, loading the includes the rendered
/// columns declared. A concurrent mutation landing between the passes can only
/// fill the window or push the second past the cap — that aborts the stream
/// loudly instead of truncating silently. The header leads the body even when
/// the window is empty, so an empty table downloads a valid CSV
/// rather than a 0-byte file indistinguishable from a failed download.
/// Formula cells are defused per OWASP in [`Table::csv_row`], and `?bom=1`
/// prepends a UTF-8 BOM for Excel interop.
pub(crate) fn resource_export<R: Resource>(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        gate::<R>(cx)?;
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        let state = TableState::from_cx(cx);
        let table = R::table(cx);
        // Fail closed on unapplied filters: a typo'd `?filters=`
        // must not silently export the unfiltered table.
        if !table.unapplied_filters(&state).is_empty() {
            return Err(topcoat::router::error::bad_request(format!(
                "invalid filters: {}",
                table
                    .unapplied_filters(&state)
                    .iter()
                    .map(|(pair, reason)| format!("{pair} ({reason})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .into());
        }
        // Phase 1: bounded visibility scan — count receivable rows inside the
        // raw cap window, so the 413 below fires before any response bytes.
        // The scan reads no column, so it passes an empty include set.
        let mut chunker = ExportChunker::new(export_base_query::<R>(
            cx,
            &table,
            &state,
            &crate::resource::IncludeNeeds::default(),
        )?);
        let mut db_handle = db(cx);
        let mut visible = 0usize;
        while let Some(rows) = chunker.next_chunk(&mut db_handle).await? {
            visible += rows.iter().filter(|r| R::can_view(cx, r)).count();
        }
        // The cap counts viewable rows, but only inside the raw window: rows
        // left past it may be viewable too, so a 200 would be a silent
        // truncation.
        if chunker.beyond_window() {
            return Err(topcoat::router::error::content_too_large().into());
        }
        enforce_export_cap_count(visible)?;
        // Phase 2: re-walk the window, streaming CSV fragments into a bounded
        // channel the response body reads from (chunked, no Content-Length).
        // The walk moves onto a spawned task with an owned `Cx` clone, so a
        // slow consumer back-pressures the fetch instead of holding the
        // handler — and never an unbounded buffer.
        //
        // The window is walked twice on purpose: the row cap must be answered
        // before the first response byte, and the cap counts rows through
        // `R::can_view`, a Rust predicate no `COUNT(*)` can run. Trimming the
        // streaming pass to the cap instead would ship a partial file for a
        // table the caller may not receive in full; the two walks are
        // the price of a fail-closed 413. This is not a truncating `LIMIT 200`.
        let want_bom = export_wants_bom(cx);
        let needs = table.include_needs();
        let (tx, body) = http_body_util::Channel::<bytes::Bytes, std::io::Error>::new(8);
        let cx2 = cx.clone();
        tokio::spawn(async move {
            let mut tx = tx;
            // The tenant scope was already resolved for phase 1 against the
            // same `cx`, table and state, so this cannot fail again — but a
            // stream that cannot build its query aborts instead of sending a
            // truncated CSV (moved the seed behind a `Result`).
            let mut chunker = match export_base_query::<R>(&cx2, &table, &state, &needs) {
                Ok(query) => ExportChunker::new(query),
                Err(error) => {
                    tracing::error!(resource = R::slug(), error = %error, "export stream failed");
                    tx.abort(std::io::Error::other("export unavailable"));
                    return;
                }
            };
            let mut db_handle = crate::db::db(&cx2);
            // The header leads the body even when the window has no rows
            // an empty table must download a valid CSV, and a
            // 0-byte body cannot be told from a failed download.
            let mut head = table.csv_header();
            // Opt-in BOM for Excel: `?bom=1` prepends U+FEFF
            // so non-ASCII cells open correctly; default stays
            // BOM-free so existing clients/tests see plain UTF-8.
            if want_bom {
                head.insert(0, '\u{FEFF}');
            }
            if tx.send_data(bytes::Bytes::from(head)).await.is_err() {
                return;
            }
            let mut visible = 0usize;
            loop {
                let rows = match chunker.next_chunk(&mut db_handle).await {
                    Ok(Some(rows)) => rows,
                    Ok(None) => break,
                    Err(error) => {
                        tracing::error!(resource = R::slug(), error = %error, "export stream failed");
                        tx.abort(std::io::Error::other("export unavailable"));
                        return;
                    }
                };
                let mut fragment = String::new();
                for row in &rows {
                    if R::can_view(&cx2, row) {
                        visible += 1;
                        if visible > MAX_EXPORT_ROWS {
                            // TOCTOU overrun: rows changed between the scan
                            // and this pass. Abort loudly — never truncate a
                            // 200 CSV silently.
                            tracing::error!(
                                resource = R::slug(),
                                "export overflowed its cap mid-stream"
                            );
                            tx.abort(std::io::Error::other("export overflowed its cap"));
                            return;
                        }
                        fragment.push_str(&table.csv_row(row));
                    }
                }
                if !fragment.is_empty() && tx.send_data(bytes::Bytes::from(fragment)).await.is_err()
                {
                    return;
                }
            }
            if chunker.beyond_window() {
                // Rows inserted between the passes can fill the window here
                // only, so phase 1's answer no longer holds.
                tracing::error!(resource = R::slug(), "export window overflowed mid-stream");
                tx.abort(std::io::Error::other("export overflowed its window"));
            }
        });
        let filename = export_filename(&R::slug());
        let res = http::Response::builder()
            .status(200)
            .header(http::header::CONTENT_TYPE, "text/csv; charset=utf-8")
            .header("x-content-type-options", "nosniff")
            .header(
                http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .body(Body::new(body))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(res)
    })
}

/// The export's filtered + ordered base query: the resource's
/// [`export_query`](crate::resource::Resource::export_query) — the soft-delete /
/// row-level seam (ADR-0002) narrowed to the relations `needs` asks for
/// and tenant-scoped by the framework on the way in
/// ([`crate::resource::scoped_query_with`]) — with the table's
/// declaration applied through the one shared routine the list loader uses.
/// The visibility scan passes an empty `needs`; the streaming pass passes
/// [`Table::include_needs`], the relations its columns declared.
///
/// The list and the export differ only in the seed query and the ordering mode:
/// the list loads the tenant-scoped `Resource::query_with` with
/// [`OrderMode::List`], the export the tenant-scoped narrowed `export_query`
/// with [`OrderMode::Export`] — whose PK fallback applies whether or not the
/// table paginates, because the chunked cursor walk needs a deterministic order
/// either way. Both pay the same scope, so a gated resource cannot export
/// unscoped.
fn export_base_query<R: Resource>(
    cx: &Cx,
    table: &Table<R::Model>,
    state: &TableState,
    needs: &crate::resource::IncludeNeeds,
) -> Result<toasty::stmt::Query<toasty::stmt::List<R::Model>>> {
    let seed = crate::resource::apply_tenant_scope::<R>(cx, R::export_query(cx, needs))?;
    Ok(table.apply_declaration(seed, state, OrderMode::Export))
}

/// One cursor-chunked pass over an export base query.
///
/// Yields the raw `MAX_EXPORT_ROWS + 1` window (the bounded over-fetch the
/// cap counts within) a chunk at a time and stops at a short chunk,
/// so callers hold one chunk instead of the window. Each chunk after the
/// first resumes from the previous chunk's `next_cursor`; a chunk shorter
/// than the requested size ends the walk, because chaining an absent cursor
/// or re-fetching cursor-free would rescan from the start. A full window is
/// probed one row past itself, so [`beyond_window`](Self::beyond_window)
/// distinguishes "the table ended" from "the window did".
struct ExportChunker<M> {
    query: toasty::stmt::Query<toasty::stmt::List<M>>,
    after: Option<toasty_core::stmt::Value>,
    raw_scanned: usize,
    exhausted: bool,
    beyond_window: bool,
}

impl<M> ExportChunker<M>
where
    M: toasty::schema::Model + Send + Sync + 'static,
{
    fn new(query: toasty::stmt::Query<toasty::stmt::List<M>>) -> Self {
        Self {
            query,
            after: None,
            raw_scanned: 0,
            exhausted: false,
            beyond_window: false,
        }
    }

    /// Whether the window filled with rows left past it, so the raw
    /// walk stopped early rather than at the table's end.
    fn beyond_window(&self) -> bool {
        self.beyond_window
    }

    async fn next_chunk(&mut self, db: &mut toasty::Db) -> Result<Option<Vec<M>>, topcoat::Error> {
        if self.exhausted {
            return Ok(None);
        }
        let remaining = (MAX_EXPORT_ROWS + 1).saturating_sub(self.raw_scanned);
        if remaining == 0 {
            // The window is full: one probe row past it says whether
            // the walk stopped on the table's end or on the window's. A full
            // chunk without a cursor cannot be probed and is treated
            // as the end.
            if let Some(cursor) = self.after.clone() {
                let probe = toasty::stmt::Paginate::new(self.query.clone(), 1)
                    .after(cursor)
                    .exec(db)
                    .await
                    .map_err(crate::db::unavailable)?;
                self.beyond_window = !probe.items.is_empty();
            }
            self.exhausted = true;
            return Ok(None);
        }
        let take = remaining.min(EXPORT_CHUNK_ROWS);
        let mut page = toasty::stmt::Paginate::new(self.query.clone(), take);
        if let Some(cursor) = self.after.take() {
            page = page.after(cursor);
        }
        let loaded = page.exec(db).await.map_err(crate::db::unavailable)?;
        if loaded.items.is_empty() {
            self.exhausted = true;
            return Ok(None);
        }
        let full = loaded.items.len() == take;
        self.raw_scanned += loaded.items.len();
        self.after = loaded.next_cursor;
        let items = loaded.items;
        if !full {
            // Short chunk: the table is exhausted — chaining the (absent)
            // cursor, or re-fetching cursor-free, would rescan from the
            // start, so stop here.
            self.exhausted = true;
            self.after = None;
        }
        Ok(Some(items))
    }
}

/// Relationship option search endpoint (D2/D5).
///
/// `GET {parent_list_url}/options?field=&q=` — server-side narrowing for
/// tables above the option cap. `field` allow-lists to a declared searchable
/// relationship `Select` in `R::form(cx)` (400 otherwise); non-searchable
/// selects keep today's cap error and never call here. `q` is trimmed and
/// clamped to the shared query bound; empty `q` returns the bounded head.
///
/// Gates: `enforce_auth` + `enforce_tenant::<R>` (parent), then the related
/// gates inside the search (`can_view_any` + tenant + `can_view` filtering
/// before labels). Parent form policy (`can_create` / `can_view`+`can_update`)
/// stays on the form pages themselves: requiring parent `can_view_any` here
/// would lock create-only users out of a form they may use, and adds no
/// visibility the related list does not already expose. `Denied` → 403, driver failure → 500,
/// filtered overflow → 200 with a "keep typing" hint option (client keeps its hint element).
/// Success → 200 `text/html` with `<option>` markup, bounded to
/// `MAX_RELATIONSHIP_OPTIONS`, values are typed PK strings, labels escaped.
pub(crate) fn resource_options<R: Resource>(cx: &Cx, _body: Body) -> RouteFuture<'_> {
    Box::pin(async move {
        gate::<R>(cx)?;
        let (field, q) = options_query(cx);
        let field = field.trim();
        if field.is_empty() {
            return Err(topcoat::router::error::bad_request("missing field").into());
        }
        let q = clamp_query_term(&q);
        let form = R::form(cx);
        let selects = form.select_inputs();
        let Some(select) = selects.get(field) else {
            return Err(topcoat::router::error::bad_request("unknown field").into());
        };
        if !select.is_relationship() {
            return Err(topcoat::router::error::bad_request("not a relationship select").into());
        }
        if !select.is_searchable() {
            return Err(topcoat::router::error::bad_request("not searchable").into());
        }
        match select.search_options(cx, &q).await {
            Ok(opts) => {
                let mut html = String::with_capacity(opts.len() * 32);
                for (v, lab) in opts {
                    html.push_str(&format!(
                        "<option value=\"{}\">{}</option>",
                        escape_option(&v),
                        escape_option(&lab)
                    ));
                }
                let res = http::Response::builder()
                    .status(200)
                    .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header("x-content-type-options", "nosniff")
                    .body(Body::from(html))
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                Ok(res)
            }
            Err(OptionLoadError::Denied) => Err(forbidden().into()),
            Err(OptionLoadError::LoadFailed) => Err(topcoat::Error::from(std::io::Error::other(
                "option search failed",
            ))),
            // Permanent: the related resource cannot be scoped at
            // all, so the search cannot succeed until the declaration is
            // fixed — a 500 that says so, not a retry.
            Err(OptionLoadError::Misdeclared) => Err(topcoat::Error::from(std::io::Error::other(
                "option search unavailable: the related resource requires a tenant the                      framework cannot scope (GH #223)",
            ))),
            Err(OptionLoadError::Overflow) => {
                let html = "<option value=\"\" disabled>Too many results — keep typing</option>"
                    .to_string();
                let res = http::Response::builder()
                    .status(200)
                    .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header("x-content-type-options", "nosniff")
                    .body(Body::from(html))
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                Ok(res)
            }
        }
    })
}

/// Parse `?field=` + `?q=` for the options endpoint (first-wins, like the
/// table state parser).
fn options_query(cx: &Cx) -> (String, String) {
    let Some(parts) = topcoat::context::try_request_context::<http::request::Parts>(cx) else {
        return (String::new(), String::new());
    };
    let query = parts.uri.query().unwrap_or("");
    let mut field = String::new();
    let mut q = String::new();
    let mut seen_field = false;
    let mut seen_q = false;
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        if k == "field" && !seen_field {
            field = v.into_owned();
            seen_field = true;
        } else if k == "q" && !seen_q {
            q = v.into_owned();
            seen_q = true;
        }
    }
    (field, q)
}

/// Escape a value/label for `<option>` markup.
fn escape_option(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use toasty::Db;

    use super::{super::Panel, *};
    use crate::panel::test_support::{Dummy, dummy_table, panel_for};
    /// The minimal table-backed model most of this module's tests share
    /// it was declared nine times, byte-identically, inside the test
    /// bodies. The resources that use it differ; the table does not.
    /// Seed `rows` dummies in a single batched insert.
    ///
    /// The export-cap tests seed more rows than a per-row `toasty::create!`
    /// loop can afford, so `create_many` accumulates the inserts into one
    /// statement. `name` maps a row index to its label.
    async fn seed_dummies(db: &mut Db, rows: usize, name: impl Fn(usize) -> String) {
        let mut create = Dummy::create_many();
        for i in 0..rows {
            create = create.item(Dummy::create().name(name(i)));
        }
        create.exec(&mut *db).await.unwrap();
    }

    #[test]
    fn parse_bulk_ids_dedupes_and_trims() {
        assert!(parse_bulk_ids("", MAX_BULK_IDS).is_empty());
        assert_eq!(
            parse_bulk_ids("a, b ,a,, c", MAX_BULK_IDS),
            vec!["a", "b", "c"]
        );
        // The cap bounds the parse too: stop at max + 1 for the handler's 400.
        assert_eq!(parse_bulk_ids("a,b,c,d,e", 3).len(), 4);
    }

    #[tokio::test]
    async fn delete_and_bulk_delete_require_can_view() {
        // the edit contract extends to deletes — a record that
        // cannot be viewed cannot be deleted by UUID-guessing the route,
        // even with `can_delete == true`.

        use crate::resource::Resource;

        struct ViewDeniedResource;
        impl Resource for ViewDeniedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                false
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = panel_for::<ViewDeniedResource>(db)
            .build()
            .expect("panel builds");
        let token = uuid::Uuid::new_v4().to_string();
        let post = |uri: String, body: String| {
            router.handle(
                http::Request::builder()
                    .uri(uri)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
        };
        // Single delete: view-denied is 403 despite can_delete == true.
        let single = post(
            format!("/admin/dummies/{}/delete", row.id),
            format!("confirm=1&csrf_token={token}"),
        )
        .await;
        assert_eq!(
            single.status(),
            http::StatusCode::FORBIDDEN,
            "view-denied single delete must 403, got {}",
            single.status()
        );
        // Bulk delete: same rule, per row.
        let bulk = post(
            "/admin/dummies/bulk-delete".to_string(),
            format!("ids={}&confirm=1&csrf_token={token}", row.id),
        )
        .await;
        assert_eq!(
            bulk.status(),
            http::StatusCode::FORBIDDEN,
            "view-denied bulk delete must 403, got {}",
            bulk.status()
        );
    }

    #[tokio::test]
    async fn bulk_delete_caps_ids_and_ignores_display_key() {
        use crate::resource::Resource;

        struct UpperKeyResource;
        impl Resource for UpperKeyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                // Non-canonical display key: bulk must still resolve
                // via the typed PK fetch alone. The record key stays canonical
                // The renderer emits it for bulk values, so the
                // display/URL split is exercised, not bypassed.
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string().to_uppercase())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            async fn bulk_delete_records(
                _cx: &Cx,
                _records: Vec<Dummy>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = panel_for::<UpperKeyResource>(db)
            .build()
            .expect("panel builds");
        // Canonical lowercase id succeeds despite uppercase Table::id.
        let token = uuid::Uuid::new_v4().to_string();
        let ok = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!(
                        "ids={}&confirm=1&csrf_token={token}",
                        row.id
                    )))
                    .unwrap(),
            )
            .await;
        assert!(
            ok.status().is_redirection(),
            "PK-authenticated bulk must not 404 on display-key mismatch, got {}",
            ok.status()
        );
        // Over-cap batch is a clear 400 before any DB work.
        let big = (0..(MAX_BULK_IDS + 1))
            .map(|i| format!("00000000-0000-0000-0000-{:012}", i))
            .collect::<Vec<_>>()
            .join(",");
        let capped = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!(
                        "ids={big}&confirm=1&csrf_token={token}"
                    )))
                    .unwrap(),
            )
            .await;
        assert_eq!(capped.status(), http::StatusCode::BAD_REQUEST);
        // Missing token is 403.
        let no_token = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::from(format!("ids={}", row.id)))
                    .unwrap(),
            )
            .await;
        assert_eq!(no_token.status(), http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_resolves_record_key_not_display_key() {
        // GH #168 defect 1 round-trip: `Table::id` projects a non-PK value
        // (the name), `Table::pk` carries the typed PK. Handlers must 404
        // the display value and accept the record key, for single and bulk.

        use crate::resource::Resource;

        struct NameKeyResource;
        impl Resource for NameKeyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.name.clone())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            async fn delete_record(
                _cx: &Cx,
                _record: Dummy,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
            async fn bulk_delete_records(
                _cx: &Cx,
                _records: Vec<Dummy>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = panel_for::<NameKeyResource>(db)
            .build()
            .expect("panel builds");
        let token = uuid::Uuid::new_v4().to_string();
        let post = |uri: String, body: String| {
            router.handle(
                http::Request::builder()
                    .uri(uri)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
        };
        // Single delete with the display value 404s — it is not a PK.
        let display_single = post(
            "/admin/dummies/Ada/delete".to_string(),
            format!("confirm=1&csrf_token={token}"),
        )
        .await;
        assert_eq!(
            display_single.status(),
            http::StatusCode::NOT_FOUND,
            "display key must not resolve, got {}",
            display_single.status()
        );
        // Single delete with the record key succeeds.
        let record_single = post(
            format!("/admin/dummies/{}/delete", row.id),
            format!("confirm=1&csrf_token={token}"),
        )
        .await;
        assert!(
            record_single.status().is_redirection(),
            "record key must delete, got {}",
            record_single.status()
        );
        // Bulk with the display value 404s.
        let display_bulk = post(
            "/admin/dummies/bulk-delete".to_string(),
            format!("ids=Ada&confirm=1&csrf_token={token}"),
        )
        .await;
        assert_eq!(
            display_bulk.status(),
            http::StatusCode::NOT_FOUND,
            "display key must not resolve in bulk, got {}",
            display_bulk.status()
        );
        // Bulk with the record key succeeds.
        let record_bulk = post(
            "/admin/dummies/bulk-delete".to_string(),
            format!("ids={}&confirm=1&csrf_token={token}", row.id),
        )
        .await;
        assert!(
            record_bulk.status().is_redirection(),
            "record key must bulk-delete, got {}",
            record_bulk.status()
        );
    }

    #[tokio::test]
    async fn bulk_delete_mid_loop_failure_deletes_zero_rows() {
        // GH #84 acceptance: fetch, policy checks, and deletes share one
        // framework transaction — an impl that fails halfway rolls everything
        // back instead of half-applying.

        use crate::resource::Resource;

        struct FlakyBulkResource;
        impl Resource for FlakyBulkResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
            async fn bulk_delete_records(
                _cx: &Cx,
                records: Vec<Dummy>,
                ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                // Delete the first row, then blow up: without the
                // framework tx the first delete would stick.
                let first = records.into_iter().next().unwrap();
                Dummy::filter(Dummy::fields().id().eq(first.id))
                    .delete()
                    .exec(&mut *ex)
                    .await
                    .map_err(topcoat::Error::from)?;
                Err(std::io::Error::other("boom").into())
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["one", "two"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let mut db_ids = db.clone();
        let rows = Dummy::all().exec(&mut db_ids).await.unwrap();
        assert_eq!(rows.len(), 2);
        let ids = rows
            .iter()
            .map(|r| r.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let router = panel_for::<FlakyBulkResource>(db.clone())
            .build()
            .expect("panel builds");
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/bulk-delete")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!(
                        "ids={ids}&confirm=1&csrf_token={token}"
                    )))
                    .unwrap(),
            )
            .await;
        assert!(
            resp.status().is_server_error(),
            "mid-loop failure must error, got {}",
            resp.status()
        );
        let rows = Dummy::all().exec(&mut db_ids).await.unwrap();
        assert_eq!(
            rows.len(),
            2,
            "rollback must leave zero rows deleted, got {}",
            2 - rows.len()
        );
    }

    #[tokio::test]
    async fn export_drops_rows_failing_can_view() {
        use http_body_util::BodyExt;

        use crate::resource::Resource;

        struct RowPolicyResource;
        impl Resource for RowPolicyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Dummy) -> bool {
                record.name != "denied"
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["allowed", "denied"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = panel_for::<RowPolicyResource>(db)
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let csv = String::from_utf8_lossy(&body);
        assert!(
            csv.contains("allowed"),
            "export must keep viewable rows, got {csv}"
        );
        assert!(
            !csv.contains("denied"),
            "export must not exceed row visibility (GH #86), got {csv}"
        );
    }

    /// the export's base query is
    /// [`Resource::export_query`](crate::resource::Resource::export_query),
    /// handed the includes the rendered columns declared — not everything
    /// [`Resource::query`](crate::resource::Resource::query) loads.
    ///
    /// Two resources over one model differ only in the declaration: both
    /// render a column that reads `parent`, both implement the same narrowed
    /// `export_query`, and only one column declares `.needs(["parent"])`. The
    /// cell therefore reports which query the export actually ran — the
    /// declared include loads the parent, the silent one does not.
    #[tokio::test]
    async fn export_query_narrows_to_the_declared_column_includes() {
        use http_body_util::BodyExt;
        use toasty::stmt::{Include, List, Query};

        use crate::resource::{IncludeNeeds, Resource};

        #[derive(Debug, toasty::Model, Clone)]
        struct Parent {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct Child {
            #[key]
            #[auto]
            id: uuid::Uuid,
            label: String,
            #[index]
            parent_id: uuid::Uuid,
            #[belongs_to(key = parent_id, references = id)]
            parent: toasty::Deferred<Parent>,
        }

        fn with_parent() -> Query<List<Child>> {
            let inc: Include<Child, Parent> = Child::fields().parent().into();
            Query::<List<Child>>::all().include(inc)
        }

        /// The narrowed base query the export asks for: the parent comes along
        /// only when a rendered column declared it.
        fn narrowed(_cx: &Cx, needs: &IncludeNeeds) -> Query<List<Child>> {
            if needs.wants("parent") {
                with_parent()
            } else {
                Query::<List<Child>>::all()
            }
        }

        struct ExportResource<const DECLARES: bool>;
        impl<const DECLARES: bool> Resource for ExportResource<DECLARES> {
            type Model = Child;
            fn slug() -> String {
                if DECLARES { "declared" } else { "bare" }.to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Child) -> bool {
                true
            }
            // The list/detail base query always loads the parent; the export
            // narrows to the declaration.
            fn query(_cx: &Cx) -> Query<List<Child>> {
                with_parent()
            }
            fn export_query(cx: &Cx, needs: &IncludeNeeds) -> Query<List<Child>> {
                narrowed(cx, needs)
            }
            fn table(cx: &Cx) -> crate::resource::Table<Child> {
                let column = crate::resource::TextColumn::computed("Parent", |c: &Child| {
                    if c.parent.is_unloaded() {
                        "(unloaded)".to_string()
                    } else {
                        c.parent.get().name.clone()
                    }
                });
                let column = if DECLARES {
                    column.needs(["parent"])
                } else {
                    column
                };
                crate::resource::Table::r#for(cx)
                    .id(|c: &Child| c.id.to_string())
                    .pk(|c: &Child| c.id.to_string())
                    .columns(column)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Parent, Child))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let parent_id = uuid::Uuid::new_v4();
        toasty::create!(Parent {
            id: parent_id,
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Child {
            label: "row".to_string(),
            parent_id,
        })
        .exec(&mut db)
        .await
        .unwrap();

        let router = Panel::new("admin")
            .app_context(db)
            .resource::<ExportResource<true>>()
            .resource::<ExportResource<false>>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        let csv = |body: bytes::Bytes| String::from_utf8_lossy(&body).into_owned();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/declared/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let declared = csv(resp.into_body().collect().await.unwrap().to_bytes());
        assert!(
            declared.contains("Ada"),
            "a declared include must reach the export query, got {declared}"
        );
        // The discriminating half: the same cell would read `(unloaded)` had
        // the export asked for the un-narrowed `query`.
        assert!(
            !declared.contains("(unloaded)"),
            "a declared include must be loaded, got {declared}"
        );

        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/bare/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let bare = csv(resp.into_body().collect().await.unwrap().to_bytes());
        assert!(
            bare.contains("(unloaded)"),
            "an include no column declared must not be loaded, got {bare}"
        );
    }

    #[test]
    fn export_bom_flag_reads_bom_query_param() {
        use topcoat::context::CxTestBuilder;

        fn cx_for(uri: &str) -> Cx {
            let (parts, ()) = http::Request::builder()
                .uri(uri)
                .body(())
                .unwrap()
                .into_parts();
            CxTestBuilder::new().request_context(parts).build()
        }

        assert!(export_wants_bom(&cx_for("/admin/users/export?bom=1")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export?bom=0")));
        assert!(!export_wants_bom(&cx_for("/admin/users/export?BOM=1")));
    }

    #[test]
    fn export_cap_maps_one_row_past_the_limit_to_413() {
        // the cap branch must produce a content-too-large error, not
        // just a constant that happens to equal 10_000. Exercised at the
        // boundary.
        enforce_export_cap_count(MAX_EXPORT_ROWS).unwrap();
        let err = enforce_export_cap_count(MAX_EXPORT_ROWS + 1).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "cap must map to content-too-large (413), got {err}"
        );
    }

    #[tokio::test]
    async fn export_streams_csv_in_chunks_with_parity() {
        // the streamed body reassembles byte-for-byte to the
        // buffered CSV (header + rows, BOM variant included), arrives without
        // a Content-Length (chunked), and multi-chunk tables cross chunk
        // boundaries without repeating or dropping rows.

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        struct ChunkedResource;
        impl Resource for ChunkedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        // 2 * chunk + a tail: crosses two chunk boundaries (500/500/203).
        let total = 2 * EXPORT_CHUNK_ROWS + 203;
        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..total {
            toasty::create!(Dummy {
                name: format!("user-{i:05}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = panel_for::<ChunkedResource>(db)
            .build()
            .expect("panel builds");
        let get_csv = async |uri: &str| {
            let resp = router
                .handle(
                    http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert!(
                resp.status().is_success(),
                "export {uri} failed: {}",
                resp.status()
            );
            let content_length = resp.headers().get(http::header::CONTENT_LENGTH).cloned();
            assert!(
                content_length.is_none(),
                "streamed export must not set Content-Length, got {content_length:?}"
            );
            assert_eq!(
                resp.headers()
                    .get(http::header::CONTENT_TYPE)
                    .map(|v| v.to_str().unwrap_or("")),
                Some("text/csv; charset=utf-8")
            );
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            String::from_utf8(body.to_vec()).unwrap()
        };
        let csv = get_csv("/admin/dummies/export").await;
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some("Name"));
        let mut names: Vec<&str> = lines.collect();
        assert_eq!(names.len(), total);
        // The export pins PK order when no sortable column is declared
        // (: cursor chunks need a deterministic order), so compare as
        // a set — chunking must neither drop nor repeat rows.
        names.sort_unstable();
        let mut expected: Vec<String> = (0..total).map(|i| format!("user-{i:05}")).collect();
        expected.sort();
        assert_eq!(
            names,
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        // BOM variant: same rows, FEFF-prefixed.
        let bom = get_csv("/admin/dummies/export?bom=1").await;
        assert!(bom.starts_with('\u{FEFF}'), "BOM must lead, got {bom:?}");
        assert_eq!(&bom['\u{FEFF}'.len_utf8()..], csv);
    }

    #[tokio::test]
    async fn export_of_an_empty_table_emits_the_header() {
        // the header leads the body even when the window has no rows,
        // so an empty table downloads a valid CSV (header, and the BOM when
        // asked for) instead of a 0-byte file a consumer cannot tell from a
        // failed download.

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        struct EmptyResource;
        impl Resource for EmptyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = panel_for::<EmptyResource>(db)
            .build()
            .expect("panel builds");
        let get = async |uri: &str| {
            let resp = router
                .handle(
                    http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert!(resp.status().is_success(), "export {uri} failed");
            resp.into_body().collect().await.unwrap().to_bytes()
        };

        assert_eq!(
            get("/admin/dummies/export").await.as_ref(),
            b"Name\n",
            "an empty export is the header line, not a 0-byte body"
        );
        assert_eq!(
            get("/admin/dummies/export?bom=1").await.as_ref(),
            "\u{FEFF}Name\n".as_bytes(),
            "the BOM variant leads with U+FEFF even when empty"
        );
    }

    #[tokio::test]
    async fn export_visibility_scan_asks_for_no_includes() {
        // The counting pass passes no relation includes; only the streaming
        // pass loads the ones the columns declared. `can_view` observes which
        // query loaded the row: the relation is unloaded in the scan and loaded
        // in the stream, so both counters must fire.
        use std::sync::atomic::{AtomicUsize, Ordering};

        use http_body_util::BodyExt;
        use toasty::stmt::{Include, List, Query};

        use crate::resource::{IncludeNeeds, Resource};

        static SCAN_UNLOADED: AtomicUsize = AtomicUsize::new(0);
        static STREAM_LOADED: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug, toasty::Model, Clone)]
        struct Parent {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct Child {
            #[key]
            #[auto]
            id: uuid::Uuid,
            label: String,
            #[index]
            parent_id: uuid::Uuid,
            #[belongs_to(key = parent_id, references = id)]
            parent: toasty::Deferred<Parent>,
        }

        fn with_parent() -> Query<List<Child>> {
            let inc: Include<Child, Parent> = Child::fields().parent().into();
            Query::<List<Child>>::all().include(inc)
        }

        struct ScanResource;
        impl Resource for ScanResource {
            type Model = Child;
            fn slug() -> String {
                "children".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Child) -> bool {
                if record.parent.is_unloaded() {
                    SCAN_UNLOADED.fetch_add(1, Ordering::SeqCst);
                } else {
                    STREAM_LOADED.fetch_add(1, Ordering::SeqCst);
                }
                true
            }
            fn query(_cx: &Cx) -> Query<List<Child>> {
                with_parent()
            }
            fn query_with(_cx: &Cx, needs: &IncludeNeeds) -> Query<List<Child>> {
                if needs.wants("parent") {
                    with_parent()
                } else {
                    Query::<List<Child>>::all()
                }
            }
            fn table(cx: &Cx) -> crate::resource::Table<Child> {
                crate::resource::Table::r#for(cx)
                    .id(|c: &Child| c.id.to_string())
                    .pk(|c: &Child| c.id.to_string())
                    .columns(
                        crate::resource::TextColumn::computed("Parent", |c: &Child| {
                            if c.parent.is_unloaded() {
                                "(unloaded)".to_string()
                            } else {
                                c.parent.get().name.clone()
                            }
                        })
                        .needs(["parent"]),
                    )
            }
        }

        SCAN_UNLOADED.store(0, Ordering::SeqCst);
        STREAM_LOADED.store(0, Ordering::SeqCst);
        let mut db = Db::builder()
            .models(toasty::models!(Parent, Child))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let parent_id = uuid::Uuid::new_v4();
        toasty::create!(Parent {
            id: parent_id,
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Child {
            label: "row".to_string(),
            parent_id,
        })
        .exec(&mut db)
        .await
        .unwrap();

        let router = panel_for::<ScanResource>(db).build().expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/children/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let csv = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(
            csv.contains("Ada"),
            "the stream must render the include: {csv}"
        );
        assert!(
            SCAN_UNLOADED.load(Ordering::SeqCst) > 0,
            "the counting pass must run without the column includes"
        );
        assert!(
            STREAM_LOADED.load(Ordering::SeqCst) > 0,
            "the streaming pass must load the declared include"
        );
    }

    #[tokio::test]
    async fn export_counts_only_viewable_rows_within_the_window() {
        // GH #145 (with), preserved under streaming: visibility is
        // counted before the cap inside the raw MAX+1 window, so interleaved
        // denied rows yield a 200 with the visible subset — never a 413, and
        // no count leak.

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        struct MixedResource;
        impl Resource for MixedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Dummy) -> bool {
                !record.name.starts_with("denied-")
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        seed_dummies(&mut db, MAX_EXPORT_ROWS + 1, |i| {
            if i % 2 == 0 {
                format!("allowed-{i:05}")
            } else {
                format!("denied-{i:05}")
            }
        })
        .await;
        let router = panel_for::<MixedResource>(db)
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let csv = String::from_utf8(body.to_vec()).unwrap();
        let rows: Vec<&str> = csv.lines().skip(1).collect();
        assert_eq!(rows.len(), (MAX_EXPORT_ROWS + 1).div_ceil(2));
        assert!(rows.iter().all(|r| r.starts_with("allowed-")));
        assert!(!csv.contains("denied-"));
    }

    #[tokio::test]
    async fn export_refuses_when_viewable_rows_lie_past_the_window() {
        // the cap counts viewable rows, but only inside the raw
        // window. With rows left past it, a 200 would be a partial CSV — the
        // export must refuse with the same 413 the cap uses.

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        struct WindowedResource;
        impl Resource for WindowedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Dummy) -> bool {
                !record.name.starts_with("denied-")
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        // 20 rows past the window: the viewable count inside it stays under
        // the cap, so only the presence of rows beyond it can refuse.
        seed_dummies(&mut db, MAX_EXPORT_ROWS + 1 + 20, |i| {
            if i % 2 == 0 {
                format!("allowed-{i:05}")
            } else {
                format!("denied-{i:05}")
            }
        })
        .await;
        let router = panel_for::<WindowedResource>(db)
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let status = resp.status();
        assert_eq!(
            status,
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "rows past the raw window must 413, got {status}"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_text = String::from_utf8_lossy(&body);
        assert!(
            !body_text.contains("allowed-") && !body_text.contains("denied-"),
            "413 must carry no CSV rows, got {body_text:?}"
        );
    }

    #[tokio::test]
    async fn export_chunker_stops_at_a_short_chunk() {
        // a short chunk ends the walk — re-fetching cursor-free
        // would rescan from the start and multiply the visible count past
        // the cap.

        use crate::resource::Resource;

        struct TinyResource;
        impl Resource for TinyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["Ada", "Bob", "Cara"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = topcoat::context::CxTestBuilder::new()
            .app_context(db.clone())
            .build();
        let table = TinyResource::table(&cx);
        let state = crate::resource::TableState::default();
        let mut chunker = ExportChunker::new(
            export_base_query::<TinyResource>(&cx, &table, &state, &table.include_needs())
                .expect("tenant scope"),
        );
        let first = chunker
            .next_chunk(&mut db)
            .await
            .unwrap()
            .expect("short first chunk");
        assert_eq!(first.len(), 3);
        assert!(
            chunker.next_chunk(&mut db).await.unwrap().is_none(),
            "short chunk must end the walk, not rescan"
        );
        assert!(
            chunker.next_chunk(&mut db).await.unwrap().is_none(),
            "exhausted walk stays exhausted"
        );
    }

    #[tokio::test]
    async fn export_and_list_agree_on_rows_and_order() {
        // both loaders apply the table declaration through one shared
        // routine, so a search term, a filter and a sort cannot reach the list
        // and miss the CSV. This drives the same state through both — the list
        // through `Table::load`, the export through `export_base_query` — and
        // compares the rows and their order.
        use std::collections::HashMap;

        use crate::resource::{OrderMode, Resource, SelectFilter, Sort, TableState, TextColumn};

        #[derive(Debug, Clone, toasty::Model)]
        struct Task {
            #[key]
            #[auto]
            id: uuid::Uuid,
            title: String,
            status: String,
        }
        struct TaskResource;
        impl Resource for TaskResource {
            type Model = Task;
            fn slug() -> String {
                "tasks".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Task) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Task> {
                crate::resource::Table::r#for(cx)
                    .id(|t: &Task| t.id.to_string())
                    .columns(
                        TextColumn::r#for(Task::fields().title(), |t: &Task| t.title.clone())
                            .searchable()
                            .sortable(),
                    )
                    .filters(SelectFilter::r#for(
                        Task::fields().status(),
                        vec!["published".to_string(), "draft".to_string()],
                    ))
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Task))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for (title, status) in [
            ("alpha", "draft"),
            ("bravo", "published"),
            ("charlie", "published"),
            ("delta", "draft"),
            ("echo", "published"),
        ] {
            toasty::create!(Task {
                title: title.to_string(),
                status: status.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = topcoat::context::CxTestBuilder::new()
            .app_context(db.clone())
            .build();

        // All three declaration steps at once: a search term, a filter and a
        // sort. `?sort=` names the declared sortable column, so both loaders
        // resolve the same ordering without the PK fallback.
        let state = TableState {
            search: Some("a".to_string()),
            filters: HashMap::from([("status".to_string(), "published".to_string())]),
            sort: Some(Sort {
                column: "title".to_string(),
                descending: true,
            }),
            ..TableState::default()
        };

        let table = TaskResource::table(&cx);
        let listed: Vec<String> = table
            .load(&cx, TaskResource::query(&cx), &state)
            .await
            .unwrap()
            .rows
            .iter()
            .map(|t| t.title.clone())
            .collect();
        assert_eq!(
            listed,
            ["charlie".to_string(), "bravo".to_string()],
            "the seed must exercise search + filter + sort"
        );

        let mut chunker = ExportChunker::new(
            export_base_query::<TaskResource>(&cx, &table, &state, &table.include_needs())
                .expect("tenant scope"),
        );
        let mut exported: Vec<String> = Vec::new();
        while let Some(rows) = chunker.next_chunk(&mut db).await.unwrap() {
            exported.extend(rows.iter().map(|t| t.title.clone()));
        }
        assert_eq!(
            listed, exported,
            "the export must agree with the list on rows and order"
        );

        // The two modes differ only where they must: an unordered table pins
        // the export to the PK, while the list keeps the query unordered.
        let unsorted = crate::resource::Table::<Task>::r#for(&cx)
            .id(|t: &Task| t.id.to_string())
            .columns(TextColumn::r#for(Task::fields().title(), |t: &Task| {
                t.title.clone()
            }));
        let neutral = TableState::default();
        assert!(
            unsorted.order_bys_for(&neutral, OrderMode::List).is_empty(),
            "an unpaginated list keeps its query unordered"
        );
        assert_eq!(
            unsorted.order_bys_for(&neutral, OrderMode::Export).len(),
            1,
            "the chunked export walk needs a deterministic order"
        );
    }

    #[tokio::test]
    async fn export_413s_above_the_cap_before_streaming() {
        // GH #172 decision 2: the MAX_EXPORT_ROWS cap stays as the backstop
        // above streaming — decided by the pre-body visibility scan, so the
        // 413 carries no partial CSV.

        use crate::resource::Resource;

        struct CappedResource;
        impl Resource for CappedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                dummy_table(cx)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        seed_dummies(&mut db, MAX_EXPORT_ROWS + 1, |i| format!("user-{i:05}")).await;
        let router = panel_for::<CappedResource>(db)
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "one row past the cap must 413"
        );
        use http_body_util::BodyExt;
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_text = String::from_utf8_lossy(&body);
        assert!(
            !body_text.contains("user-"),
            "413 must carry no CSV rows, got {body_text:?}"
        );
    }

    #[test]
    fn export_filename_cannot_split_the_disposition_header() {
        // `slug()` is an overridable free-form String, so quote and
        // control characters must never reach the Content-Disposition header.
        assert_eq!(export_filename("users"), "users.csv");
        for hostile in [
            "a\"b\r\nContent-Length: 0",
            "a\\\"b",
            "\nadmin",
            "bad\u{0}name",
        ] {
            let filename = export_filename(hostile);
            assert!(
                !filename.contains('"')
                    && !filename.contains('\\')
                    && !filename.contains('\r')
                    && !filename.contains('\n')
                    && !filename.chars().any(char::is_control),
                "hostile slug {hostile:?} must be defused, got {filename:?}"
            );
            assert!(filename.ends_with(".csv"), "suffix kept: {filename:?}");
        }
        // A slug that defuses to nothing falls back to a usable filename.
        assert_eq!(export_filename(""), "export.csv");
        assert_eq!(export_filename("\""), "export.csv");
        // Bounded header value.
        let long = "x".repeat(500);
        assert_eq!(export_filename(&long).len(), 100 + ".csv".len());
    }

    #[tokio::test]
    async fn find_by_key_loads_one_row_scoped_and_404s_malformed() {
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model, Clone)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let a = toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(Subscriber { email: "z@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        // Existing id → exactly that row (typed PK filter, not a full scan).
        let got = find_by_key::<SubscriberResource>(&cx, &a.id.to_string(), &mut ex)
            .await
            .unwrap();
        assert_eq!(got.id, a.id);

        // Well-formed but unknown id → 404.
        let missing = uuid::Uuid::new_v4().to_string();
        assert!(
            find_by_key::<SubscriberResource>(&cx, &missing, &mut ex)
                .await
                .is_err(),
            "unknown id must not resolve"
        );

        // Malformed id (not a Uuid) → 404, not a query error.
        assert!(
            find_by_key::<SubscriberResource>(&cx, "not-a-uuid", &mut ex)
                .await
                .is_err(),
            "malformed id must not resolve"
        );
    }

    #[tokio::test]
    async fn composite_pk_edit_fails_loudly_not_404() {
        // a composite-PK resource is a programming error the URL
        // scheme cannot serve — 500 with a message, never per-id 404s.

        use crate::resource::Resource;

        #[derive(Debug, Clone, toasty::Model)]
        struct Pair {
            #[key]
            a: String,
            #[key]
            b: String,
            name: String,
        }
        struct PairResource;
        impl Resource for PairResource {
            type Model = Pair;
            fn slug() -> String {
                "pairs".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Pair) -> bool {
                true
            }
            fn can_update(_cx: &Cx, _record: &Pair) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Pair> {
                crate::resource::Table::r#for(cx)
                    .id(|p: &Pair| format!("{}-{}", p.a, p.b))
                    .columns(crate::resource::TextColumn::r#for(
                        Pair::fields().name(),
                        |p: &Pair| p.name.clone(),
                    ))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Pair))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = panel_for::<PairResource>(db).build().expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/pairs/whatever/edit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "composite PK must fail loudly, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn options_endpoint_searches_and_gates() {
        // `GET {parent}/options?field=&q=` narrows server-side,
        // allow-lists to searchable relationship selects, and mirrors gates.
        use http_body_util::BodyExt;

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model, Clone)]
        struct OptAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct OptAuthorResource;
        impl Resource for OptAuthorResource {
            type Model = OptAuthor;
            fn slug() -> String {
                "opt-authors".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &OptAuthor) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<OptAuthor> {
                crate::resource::Table::r#for(cx)
                    .id(|a: &OptAuthor| a.id.to_string())
                    .pk(|a: &OptAuthor| a.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(
                            OptAuthor::fields().name(),
                            |a: &OptAuthor| a.name.clone(),
                        )
                        .searchable(),
                    )
            }
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct OptPost {
            #[key]
            #[auto]
            id: uuid::Uuid,
            author_id: uuid::Uuid,
            title: String,
        }
        struct OptPostResource;
        impl Resource for OptPostResource {
            type Model = OptPost;
            fn slug() -> String {
                "opt-posts".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &OptPost) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<OptPost> {
                crate::resource::Table::r#for(cx)
                    .id(|p: &OptPost| p.id.to_string())
                    .pk(|p: &OptPost| p.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        OptPost::fields().title(),
                        |p: &OptPost| p.title.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(
                    crate::schema::Select::r#for(OptPost::fields().author_id())
                        .relationship::<OptAuthorResource>(
                            OptAuthorResource::query,
                            |a: &OptAuthor| a.id,
                            |a: &OptAuthor| a.name.clone(),
                        )
                        .searchable(),
                )
            }
        }

        async fn body_text(resp: http::Response<Body>) -> String {
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            String::from_utf8_lossy(&bytes).to_string()
        }

        let mut db = Db::builder()
            .models(toasty::models!(OptAuthor, OptPost))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for name in ["Ada", "Grace", "Alan"] {
            toasty::create!(OptAuthor {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<OptPostResource>()
            .resource::<OptAuthorResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        // Narrowing works.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/opt-posts/options?field=author_id&q=Ada")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::OK);
        let html = body_text(resp).await;
        assert!(html.contains("Ada"), "search must return Ada, got {html}");
        assert!(!html.contains("Grace"), "search must narrow, got {html}");

        // Unknown field → 400.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/opt-posts/options?field=nope&q=Ada")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

        // Missing field → 400.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/opt-posts/options?q=Ada")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        // Escaping itself is pinned where it can fail: `escape_option`'s unit
        // test feeds characters that must be escaped and asserts the exact
        // output. These fixtures are "Ada"/"Grace"/"Alan", so a
        // `!html.contains("<script")` here could never fail.
    }

    #[tokio::test]
    async fn option_load_asks_for_no_relation_includes() {
        // an option load projects a value and a label off the related
        // record's own columns, so it asks the source for no relation
        // includes. The source's `query` loads `parent` and `can_view` keeps a
        // row only while that relation is unloaded, so a rendered option
        // proves the loader ran the needs-aware branch, not the full `query`.
        use http_body_util::BodyExt;
        use toasty::stmt::{Include, List, Query};

        use crate::resource::{IncludeNeeds, Resource};

        #[derive(Debug, toasty::Model, Clone)]
        struct Parent {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct Child {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
            #[index]
            parent_id: uuid::Uuid,
            #[belongs_to(key = parent_id, references = id)]
            parent: toasty::Deferred<Parent>,
        }

        fn with_parent() -> Query<List<Child>> {
            let inc: Include<Child, Parent> = Child::fields().parent().into();
            Query::<List<Child>>::all().include(inc)
        }

        struct ChildSource;
        impl Resource for ChildSource {
            type Model = Child;
            fn slug() -> String {
                "children".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &Child) -> bool {
                record.parent.is_unloaded()
            }
            fn query(_cx: &Cx) -> Query<List<Child>> {
                with_parent()
            }
            fn query_with(_cx: &Cx, needs: &IncludeNeeds) -> Query<List<Child>> {
                if needs.wants("parent") {
                    with_parent()
                } else {
                    Query::<List<Child>>::all()
                }
            }
            fn table(cx: &Cx) -> crate::resource::Table<Child> {
                crate::resource::Table::r#for(cx)
                    .id(|c: &Child| c.id.to_string())
                    .pk(|c: &Child| c.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Child::fields().name(),
                        |c: &Child| c.name.clone(),
                    ))
            }
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct Owner {
            #[key]
            #[auto]
            id: uuid::Uuid,
            child_id: uuid::Uuid,
            name: String,
        }

        struct OwnerResource;
        impl Resource for OwnerResource {
            type Model = Owner;
            fn slug() -> String {
                "owners".to_string()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(
                    crate::schema::Select::r#for(Owner::fields().child_id())
                        .relationship::<ChildSource>(
                            ChildSource::query,
                            |c: &Child| c.id,
                            |c: &Child| c.name.clone(),
                        )
                        .searchable(),
                )
            }
            fn table(cx: &Cx) -> crate::resource::Table<Owner> {
                crate::resource::Table::r#for(cx)
                    .id(|o: &Owner| o.id.to_string())
                    .pk(|o: &Owner| o.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Owner::fields().name(),
                        |o: &Owner| o.name.clone(),
                    ))
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Parent, Child, Owner))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let parent_id = uuid::Uuid::new_v4();
        toasty::create!(Parent {
            id: parent_id,
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Child {
            name: "Only Child".to_string(),
            parent_id,
        })
        .exec(&mut db)
        .await
        .unwrap();

        let router = Panel::new("admin")
            .app_context(db)
            .resource::<OwnerResource>()
            .resource::<ChildSource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/owners/options?field=child_id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let html = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(
            html.contains("Only Child"),
            "the option must render, which it cannot if the loader loaded the source's include: {html}"
        );
    }

    #[tokio::test]
    async fn options_endpoint_rejects_non_searchable_and_overflows() {
        // GH #150 D5/D6: non-searchable selects never serve search (400);
        // filtered overflow answers 200 with the keep-typing hint.
        use http_body_util::BodyExt;

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model, Clone)]
        struct BigA {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct BigAResource;
        impl Resource for BigAResource {
            type Model = BigA;
            fn slug() -> String {
                "big-as".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &BigA) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<BigA> {
                crate::resource::Table::r#for(cx)
                    .id(|a: &BigA| a.id.to_string())
                    .pk(|a: &BigA| a.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(BigA::fields().name(), |a: &BigA| {
                            a.name.clone()
                        })
                        .searchable(),
                    )
            }
        }

        #[derive(Debug, toasty::Model, Clone)]
        struct BigP {
            #[key]
            #[auto]
            id: uuid::Uuid,
            author_id: uuid::Uuid,
            /// A text column for the table declaration: every
            /// servable resource needs one, and `author_id` is a Uuid.
            name: String,
        }
        struct SearchableParent;
        impl Resource for SearchableParent {
            type Model = BigP;
            fn slug() -> String {
                "big-ps".to_string()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(
                    crate::schema::Select::r#for(BigP::fields().author_id())
                        .relationship::<BigAResource>(
                            BigAResource::query,
                            |a: &BigA| a.id,
                            |a: &BigA| a.name.clone(),
                        )
                        .searchable(),
                )
            }
            fn table(cx: &Cx) -> crate::resource::Table<BigP> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &BigP| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        BigP::fields().name(),
                        |r: &BigP| r.name.clone(),
                    ))
            }
        }
        struct PlainParent;
        impl Resource for PlainParent {
            type Model = BigP;
            fn slug() -> String {
                "plain-ps".to_string()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(
                    crate::schema::Select::r#for(BigP::fields().author_id())
                        .relationship::<BigAResource>(
                            BigAResource::query,
                            |a: &BigA| a.id,
                            |a: &BigA| a.name.clone(),
                        ),
                )
            }
            fn table(cx: &Cx) -> crate::resource::Table<BigP> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &BigP| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        BigP::fields().name(),
                        |r: &BigP| r.name.clone(),
                    ))
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(BigA, BigP))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..=crate::schema::MAX_RELATIONSHIP_OPTIONS {
            toasty::create!(BigA {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<SearchableParent>()
            .resource::<PlainParent>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        // Non-searchable → 400 (keeps today's cap error path, never search).
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/plain-ps/options?field=author_id&q=author-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

        // Empty q on over-cap searchable → 200 with keep-typing hint.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/big-ps/options?field=author_id&q=")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            html.contains("keep typing"),
            "filtered overflow must hint, got {html}"
        );
    }

    #[test]
    fn options_query_parses_first_wins_and_escapes() {
        assert_eq!(escape_option("a&b<c>\"'"), "a&amp;b&lt;c&gt;&quot;&#39;");
        assert_eq!(
            escape_option("550e8400-e29b-41d4-a716-446655440000"),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }
}
