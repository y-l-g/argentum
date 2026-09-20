//! Generic resource list page + live-search host + table page loader.
//!
//! One generic handler drives every resource's list; the live variant owns
//! the interaction signals and fills the grid through the `search` module's
//! shard. Shared grid helpers (`wire_table_actions`, `grid_error_view`)
//! keep the streamed page and the shard from drifting (GH #134).

use topcoat::view::internal::ThenView;
use topcoat::{
    Result,
    context::Cx,
    router::{Body, error::forbidden},
    runtime::Event,
    view::{BoxView, HoistView, ViewExt, attributes, suspense, view},
};

use super::{enforce_auth, enforce_tenant, list_url};
use crate::resource::{Resource, Table, TablePage, TableSignals, TableState};

/// Retry link for a failed streamed grid load (GH #110).
///
/// A malformed `?after=`/`?before=` cursor — or a conflicting `after` +
/// `before` pair (GH #155) — is the failure itself: retrying the
/// identical URL loops forever, so drop pagination from the link and keep the
/// rest of the state (search/sort/filters/grouping). Every other failure keeps
/// pagination too (GH #98) so a transient blip retries the same evidence.
pub(crate) fn retry_url_for_error(
    state: &TableState,
    error: &topcoat::Error,
    path: &str,
) -> String {
    if error
        .downcast_ref::<crate::cursor::CursorDecodeError>()
        .is_some()
    {
        state.without_cursor(path)
    } else {
        state.list_url(path)
    }
}

/// Delete/bulk/edit action wiring shared by the streamed list and the
/// live-search shard (GH #134): the delete form posts to `{list url}/{id}/delete`
/// and the bulk bar to `{list url}/bulk-delete`, both derived from the panel
/// declaration (not the request path) so the URLs are right wherever the table
/// renders. Read-only resources opt out via `Resource::deletable` (GH #96)
/// instead of rendering buttons that always 403.
///
/// `live` selects the shard variant: the swapped region is everything except
/// the toolbar the page owns eagerly (the live host owns those slots, so swaps
/// must never nest invocations or duplicate inputs), hence the shard forces
/// `.without_skeleton().search(false).filters(false)` while the streamed page
/// keeps the declared table as-is. The filter bar joins the search toolbar
/// there (GH #166): a control rebuilt by its own rerun loses focus.
pub(crate) fn wire_table_actions<R: Resource>(cx: &Cx, live: bool) -> Table<R::Model> {
    let mut table = R::table(cx);
    if live {
        table = table.without_skeleton().search(false).filter_bar(false);
    }
    if R::deletable() {
        table = table
            .with_delete(list_url(cx, &R::slug()))
            .with_bulk_delete(true);
    }
    if R::editable() {
        table = table.with_edit(list_url(cx, &R::slug()));
    }
    table
}

/// Branded in-region grid failure shared by the streamed list and the
/// live-search shard (GH #134, GH #158): the trace line, the cursor-aware
/// retry link ([`retry_url_for_error`]), and the `ErrorState` render are one
/// copy so the three load sites cannot drift.
///
/// On a live table (`signals`) the retry stays in place (GH #166): it writes
/// the same reset its `href` spells out — dropping pagination for a cursor
/// failure, the whole query for anything else — so recovering no longer throws
/// away signal-held state with a full navigation. `href` stays as the no-JS
/// fallback.
pub(crate) fn grid_error_view<'a, R: Resource>(
    cx: &'a Cx,
    state: &TableState,
    error: &topcoat::Error,
    path: &str,
    signals: Option<&TableSignals>,
) -> BoxView<'a> {
    tracing::error!(resource = R::slug(), error = %error, "table load failed");
    let retry = retry_url_for_error(state, error, path);
    let action: BoxView<'a> = match signals {
        Some(signals) => {
            let cursor = signals.cursor.clone();
            let none = crate::resource::cursor_none();
            let cursor_error = error
                .downcast_ref::<crate::cursor::CursorDecodeError>()
                .is_some();
            if cursor_error {
                let attrs = attributes! {
                    cx =>
                    href=(retry)
                    @click=$(|e: Event| {
                        e.prevent_default();
                        cursor.set(none.clone());
                    })
                };
                view! { cx => <a (attrs)>"Retry"</a> }.boxed()
            } else {
                let (q, filters, sort) = (
                    signals.q.clone(),
                    signals.filters.clone(),
                    signals.sort.clone(),
                );
                let attrs = attributes! {
                    cx =>
                    href=(retry)
                    @click=$(|e: Event| {
                        e.prevent_default();
                        q.set("".to_owned());
                        filters.set("".to_owned());
                        sort.set("".to_owned());
                        cursor.set(none.clone());
                    })
                };
                view! { cx => <a (attrs)>"Retry"</a> }.boxed()
            }
        }
        None => view! { cx => <a href=(retry)>"Retry"</a> }.boxed(),
    };
    view! {
        cx =>
        argentum_ui::error_state(
            title: format!("Couldn't load {}", R::navigation_label()),
            detail: "Something went wrong while loading the records.",
            action: Some(action.into()),
            attrs: attributes! { role="alert" }
        )
    }
    .boxed()
}

/// The list page every declared [`Resource`] gets at `{prefix}/{slug}`.
///
/// One generic handler drives all resources: resolve the [`TableState`] from
/// the URL, scope through `Resource::query` (the tenancy seam, ADR-0002),
/// apply the table's search/sort/pagination declarations, render through
/// `Resource::table`. The page title is the resource's navigation label.
///
/// The page streams: shell and header go out with the first content, while the
/// row grid (toolbar/filter/bulk/pager included) loads inside a `suspense`
/// region that swaps in the skeleton → table without any client-side fetching
/// (GH #98: the skeleton is thead + placeholders only, so chrome pops in with
/// the swap by design).
pub(crate) fn resource_list<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_view_any(cx) {
            return Err(forbidden().into());
        }
        // Ensure the CSRF cookie before streaming starts (GH #99): streamed
        // children can only read it via current_token.
        crate::csrf::ensure_token(cx);
        let state = TableState::from_cx(cx);
        let table = wire_table_actions::<R>(cx, false);
        let title = R::navigation_label();
        let list_path = list_url(cx, &R::slug());
        // Create entry point (GH #162, Filament's List page `CreateAction` in
        // the page header): a real link so no-JS keeps working. Gated on
        // `can_create`; the POST handler enforces it again.
        let create_url = R::can_create(cx).then(|| format!("{}/create", list_path));
        let create_label = format!("Create {}", R::navigation_label());
        if table.is_live_search() {
            return Ok(resource_list_live::<R>(cx, table, state, title, list_path));
        }

        // First content: the skeleton grid (same markup the eager
        // `defer(true)` path renders), while the rows load below. The load
        // catches its own errors: post-stream the status line is fixed,
        // so a failed load must render the branded ErrorState
        // inside the region instead of truncating the body. Pre-stream
        // failures (e.g. the skeleton itself) still propagate and map onto
        // the response status. (For children that partially stream before
        // failing, topcoat's `error_boundary` is the replace-in-place seam.)
        let skeleton = table.render_skeleton(cx).await?;
        // The swap payload must be rows even when the declared table sets
        // `.defer(true)` (GH #98 trap: render() would return a second skeleton).
        let table = table.without_skeleton();
        // Normalize once for the closure (GH #153): the retry link must not
        // echo an unknown `?group_by=`. The render re-normalizes internally.
        let state = table.normalize_state(&state);
        let lazy_rows = ThenView::new(async move {
            let grid = async {
                let page = load_table_page::<R>(cx, &table, &state).await?;
                table.render(cx, page).await
            };
            match grid.await {
                Ok(view) => Ok(view),
                Err(error) => Ok(grid_error_view::<R>(
                    cx,
                    &state,
                    &error,
                    &list_url(cx, &R::slug()),
                    None,
                )),
            }
        });

        Ok(view! {
            cx =>
            argentum_ui::page(
                argentum_ui::page_header(
                    <div class="flex items-center justify-between gap-4">
                        argentum_ui::page_title((title.clone()))
                        if let Some(url) = create_url {
                            <a
                                href=(url)
                                class=(argentum_ui::button_variants(
                                    argentum_ui::ButtonVariant::Primary,
                                    argentum_ui::ButtonSize::Md,
                                ))
                            >
                                (create_label)
                            </a>
                        }
                    </div>
                )
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        suspense(fallback: skeleton, (lazy_rows.boxed()))
                    </div>
                )
            )
        }
        .boxed())
    })))
}

/// Live list page for `Table::live_search` tables (GH #104, GH #151): the
/// page owns the interaction signals (`q`, `filters`, `sort`, `dir`,
/// `after`, `before`, `group_by`) and renders the search toolbar eagerly above the
/// streamed region while the `table_search` shard invocation fills the grid
/// below — one grid per response, so rows can never duplicate. Every
/// interaction writes a signal, so search, sort, filters, and pagination
/// re-render only the invocation output, morphing in place with focus and
/// scroll surviving.
pub(crate) fn resource_list_live<R: Resource>(
    cx: &Cx,
    table: Table<R::Model>,
    state: TableState,
    title: String,
    list_path: String,
) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        use crate::resource::TableSignals;
        use topcoat::runtime::signal;

        let signals = TableSignals {
            q: signal(cx, || state.search.clone().unwrap_or_default()),
            filters: signal(cx, || state.filters_param().unwrap_or_default()),
            sort: signal(cx, || {
                state
                    .sort
                    .as_ref()
                    .map(|s| s.column.clone())
                    .unwrap_or_default()
            }),
            dir: signal(cx, || {
                state
                    .sort
                    .as_ref()
                    .map(|s| if s.descending { "desc" } else { "asc" })
                    .unwrap_or("asc")
                    .to_string()
            }),
            cursor: signal(cx, || match (&state.after, &state.before) {
                (Some(token), _) => crate::resource::cursor_after(token),
                (None, Some(token)) => crate::resource::cursor_before(token),
                (None, None) => crate::resource::cursor_none(),
            }),
            group_by: signal(cx, || state.group_by.clone().unwrap_or_default()),
            bulk: signal(cx, String::new),
        };
        let host = if table.search_enabled() {
            Some(
                table
                    .render_live_search_bar(cx, &state, &list_path, &signals)
                    .await?,
            )
        } else {
            None
        };
        // The filter bar is hoisted next to the search host (GH #166): a
        // `<select>` change re-renders the grid, and a control inside the
        // swapped region would lose focus and collapse its popup mid-change.
        let filter_bar = if table.filter_bar_enabled() {
            Some(
                table
                    .render_live_filter_bar(cx, &state, &list_path, &signals)
                    .await?,
            )
        } else {
            None
        };
        let skeleton = table.render_skeleton(cx).await?;
        // The delete confirmation dialog is not part of the swapped grid
        // region: a keystroke starts a new result set and must never carry
        // (or re-open) a dialog, so the live page renders it eagerly once
        // (GH #151).
        let delete_dialog = table.render_delete_dialog(cx, &state, &list_path).await?;
        // Create entry point (GH #162): same header button as the streamed
        // list — a real link above the swapped region, gated on `can_create`.
        let create_url = R::can_create(cx).then(|| format!("{}/create", list_path));
        let create_label = format!("Create {}", R::navigation_label());
        // The swap payload must be rows even when the declared table sets
        // `.defer(true)` (GH #98 trap).
        let table = table.without_skeleton();
        // Normalize once for the closure (GH #153): the retry link must not
        // echo an unknown `?group_by=`. The invocation normalizes internally.
        let state = table.normalize_state(&state);
        let lazy_rows = ThenView::new(async move {
            // The retry link inside the grid writes the same signals the
            // toolbar does (GH #166), so a bad cursor recovers in place.
            let retry_signals = signals.clone();
            let grid = table
                .render_live_invocation(cx, &state, &list_path, signals)
                .await;
            match grid {
                Ok(view) => Ok(view),
                Err(error) => Ok(grid_error_view::<R>(
                    cx,
                    &state,
                    &error,
                    &list_path,
                    Some(&retry_signals),
                )),
            }
        });

        Ok(view! {
            cx =>
            argentum_ui::page(
                argentum_ui::page_header(
                    <div class="flex items-center justify-between gap-4">
                        argentum_ui::page_title((title.clone()))
                        if let Some(url) = create_url {
                            <a
                                href=(url)
                                class=(argentum_ui::button_variants(
                                    argentum_ui::ButtonVariant::Primary,
                                    argentum_ui::ButtonSize::Md,
                                ))
                            >
                                (create_label)
                            </a>
                        }
                    </div>
                )
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        if let Some(host) = host {
                            (host)
                        }
                        if let Some(bar) = filter_bar {
                            (bar)
                        }
                        suspense(fallback: skeleton, (lazy_rows.boxed()))
                        if let Some(dialog) = delete_dialog {
                            (dialog)
                        }
                    </div>
                )
            )
        })
    })))
}

/// Resolve the declared table (search / filters / sort / pagination) against
/// `Resource::query` and execute it — the data-loading half of
/// [`resource_list`], kept separate so the page shell can stream before it.
///
/// Resource lists must declare a page size (GH #172): without
/// [`Table::paginate`] the load would be an unbounded `exec`, so the missing
/// declaration fails loudly here — like a missing row key at render — instead
/// of silently loading the whole table. Page-owned tables (the showcase
/// demos, GH #154 §2) load through [`Table::load`] directly and keep the
/// unbounded branch for previews.
pub(crate) async fn load_table_page<R: Resource>(
    cx: &Cx,
    table: &Table<R::Model>,
    state: &TableState,
) -> Result<TablePage<R::Model>> {
    if table.page_size().is_none() {
        return Err(std::io::Error::other(
            "resource list requires Table::paginate(..) — unbounded tables are previews only (GH #172)",
        )
        .into());
    }
    table.load(cx, R::query(cx), state).await
}

#[cfg(test)]
mod tests {
    use super::super::Panel;
    use super::super::table_search;
    use super::*;
    use toasty::Db;

    #[tokio::test]
    async fn live_search_host_and_shard_dispatch() {
        // GH #104: opt-in tables render the signal host (page bodies are
        // hoisted, so signals work there); the slug-dispatched shard serves
        // the grid and 404s unknown paths.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
            featured: bool,
        }
        struct LiveResource;
        impl Resource for LiveResource {
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
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                            d.name.clone()
                        })
                        .searchable()
                        .sortable(),
                    )
                    .filters(crate::resource::TernaryFilter::r#for(
                        Dummy::fields().featured(),
                    ))
                    .paginate(1)
                    .live_search(true)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
            featured: false,
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db.clone())
            .resource::<LiveResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        // List page carries the live host + GET fallback.
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("data-live-search"),
            "opt-in table must render the shard host, got {html}"
        );
        // GH #166: the filter bar is hoisted next to the search host — it
        // renders eagerly, above the swapped region, so a filter change cannot
        // rebuild the control the user is interacting with.
        let filter_at = html
            .find("data-filter-name=")
            .unwrap_or_else(|| panic!("live page must render the filter bar eagerly, got {html}"));
        let swapped_at = html
            .find("topcoat::region::start")
            .unwrap_or_else(|| panic!("live page must render the streamed region, got {html}"));
        assert!(
            filter_at < swapped_at,
            "the filter bar must sit outside the swapped region, got {html}"
        );
        assert!(
            html.contains("<noscript>"),
            "live table must keep the GET fallback, got {html}"
        );

        // Shard dispatch through the real runtime endpoint (JSON args + the
        // identity header the browser sends): unknown path fails, registered
        // path renders rows and the live controls bound to the caller's
        // signals.
        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        // Args are positional shard inputs: q, filters, sort, dir, the single
        // cursor wire (GH #166), group_by, and the bulk handle the grid binds
        // its selection transport to.
        let shard_args =
            |path: &str, q: &str, filters: &str, sort: &str, dir: &str, cursor: &str| {
                format!(
                    r#"["{path}",{}, {}, {}, {}, {}, {}, {}]"#,
                    sig(1, q),
                    sig(2, filters),
                    sig(3, sort),
                    sig(4, dir),
                    sig(5, cursor),
                    sig(6, ""),
                    sig(7, "")
                )
            };
        async fn call_shard(
            router: &topcoat::router::Router,
            args: String,
        ) -> http::Response<Body> {
            let shard = topcoat::runtime::Shard::id(&table_search);
            router
                .handle(
                    http::Request::builder()
                        .method(http::Method::POST)
                        .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                        .header(http::header::CONTENT_TYPE, "application/json")
                        .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                        .body(Body::from(format!(r#"{{"args":{args},"signals":{{}}}}"#)))
                        .unwrap(),
                )
                .await
        }
        let nope = call_shard(&router, shard_args("/admin/nope", "Ada", "", "", "", "")).await;
        assert!(
            nope.status().is_client_error(),
            "unknown shard path must fail, got {}",
            nope.status()
        );
        let grid = call_shard(&router, shard_args("/admin/dummies", "Ada", "", "", "", "")).await;
        let status = grid.status();
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert_eq!(status, http::StatusCode::OK, "shard body: {grid_html}");
        assert!(
            grid_html.contains("Ada"),
            "live shard must render matching rows, got {grid_html}"
        );
        // ...and not render it a second time inside the swapped grid.
        assert!(
            !grid_html.contains("data-filter-name="),
            "the swapped grid must not duplicate the hoisted filter bar, got {grid_html}"
        );
        // GH #166: the bulk selection is signal-backed — the grid binds its
        // transport to the selection signal, so a rerun re-renders the
        // selection instead of dropping it...
        assert!(
            grid_html.contains(
                r#"data-topcoat-bind:value="(cx.hydrate({&quot;t&quot;:&quot;Signal&quot;,&quot;id&quot;:&quot;00000000000000000000000000000007&quot;})).get()""#
            ),
            "the grid must bind the bulk transport to the selection signal, got {grid_html}"
        );
        // GH #184 replaced the disabled destructive submit with a confirmation
        // dialog: the trigger is a plain button and the dialog's submit is the
        // one that carries `confirm=1` inside the same form.
        assert!(
            grid_html.contains("data-bulk-confirm-trigger")
                && grid_html.contains("data-bulk-confirm-dialog"),
            "the live grid must carry the bulk confirmation, got {grid_html}"
        );
        // ...while never reading it: selecting a row must not re-run the query.
        assert!(
            !grid_html.contains(r#"::topcoat::dep("00000000000000000000000000000007")"#),
            "the bulk signal must not become a shard dependency, got {grid_html}"
        );
        // GH #151: the grid's chrome is bound to the signals, so sort/pager
        // interactions re-render in place. `href` stays the no-JS fallback.
        assert!(
            grid_html.contains("data-topcoat-on:click") && grid_html.contains("sort=name"),
            "live grid must bind the sort link and keep its href, got {grid_html}"
        );

        // A direct context for the loader/cursor assertions below.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/dummies")
            .body(())
            .unwrap()
            .into_parts();
        let cx = topcoat::context::CxTestBuilder::new()
            .request_context(parts)
            .app_context(db)
            .build();

        // Cursors are honored as sent (GH #151): the sort/filter/search
        // handlers clear them in the browser, so a live cursor always belongs
        // to the current query; crafting one past a new query is the client's
        // own read-only inconsistency.
        let paged = crate::resource::Table::<Dummy>::r#for(&cx)
            .id(|d: &Dummy| d.id.to_string())
            .pk(|d: &Dummy| d.id.to_string())
            .columns(crate::resource::TextColumn::r#for(
                Dummy::fields().name(),
                |d: &Dummy| d.name.clone(),
            ))
            .paginate(1);
        for name in ["Bob", "Cara"] {
            toasty::create!(Dummy {
                name: name.to_string(),
                featured: false,
            })
            .exec(&mut crate::db::db(&cx))
            .await
            .unwrap();
        }
        let page1 =
            load_table_page::<LiveResource>(&cx, &paged, &crate::resource::TableState::default())
                .await
                .unwrap();
        assert_eq!(page1.rows.len(), 1);
        let first_name = page1.rows[0].name.clone();
        let wire = crate::resource::cursor_after(
            &page1
                .next_cursor
                .clone()
                .expect("page 1 must have a cursor"),
        );
        let grid = call_shard(&router, shard_args("/admin/dummies", "", "", "", "", &wire)).await;
        assert_eq!(grid.status(), http::StatusCode::OK);
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes);
        assert!(
            !grid_html.contains(&first_name),
            "a cursor must continue past page-1 rows, got {grid_html}"
        );
        // The pager renders its next/prev links with handlers too.
        assert!(
            grid_html.contains("data-topcoat-on:click"),
            "live pager must bind its cursor handlers, got {grid_html}"
        );
        // A fresh search with no cursor starts a new result set.
        let grid = call_shard(&router, shard_args("/admin/dummies", "Bob", "", "", "", "")).await;
        assert_eq!(grid.status(), http::StatusCode::OK);
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes);
        assert!(
            grid_html.contains("Bob"),
            "fresh search must match new query, got {grid_html}"
        );
    }

    #[tokio::test]
    async fn live_search_input_debounces_keystrokes() {
        // GH #172 decision 4: the visible input is unbound (keystrokes stay
        // local until the debounce delay), the hidden transport carries the
        // bound `@change` write, and the GET form survives as the no-JS
        // fallback.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct LiveResource;
        impl Resource for LiveResource {
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
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                            d.name.clone()
                        })
                        .searchable()
                        .sortable(),
                    )
                    .paginate(25)
                    .live_search(true)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<LiveResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("data-live-search-input"),
            "visible input must carry the debounce hook, got {html}"
        );
        assert!(
            html.contains("data-debounce-ms=\"200\""),
            "debounce delay must be pinned in the markup, got {html}"
        );
        assert!(
            html.contains("data-live-search-transport"),
            "hidden transport must carry the bound write, got {html}"
        );
        assert!(
            html.contains("data-topcoat-on:change"),
            "transport must write signals on change, got {html}"
        );
        assert!(
            !html.contains("data-topcoat-on:input"),
            "visible input must be unbound (debounce owns keystrokes), got {html}"
        );
        assert!(
            html.contains("<noscript>") && html.contains("name=\"q\""),
            "live table must keep the GET fallback, got {html}"
        );
    }

    #[tokio::test]
    async fn read_only_resource_hides_delete_chrome() {
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct ReadOnlyResource;
        impl Resource for ReadOnlyResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn deletable() -> bool {
                false
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
                    .paginate(25)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<ReadOnlyResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("data-bulk-form") && !html.contains("Bulk Delete"),
            "read-only list must not render bulk chrome, got {html}"
        );
        assert!(
            !html.contains("/delete"),
            "read-only list must not render delete actions, got {html}"
        );
    }

    #[tokio::test]
    async fn unpaginated_resource_list_fails_loud_without_loading() {
        // GH #172: a resource list without `Table::paginate` fails loudly in
        // the grid region instead of unbounded-loading the whole table — the
        // seeded row must not render, and the branded error state must.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct UnpaginatedResource;
        impl Resource for UnpaginatedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn deletable() -> bool {
                false
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<UnpaginatedResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(resp.status().is_success());
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Couldn't load Dummies"),
            "unpaginated list must render the error state, got {html}"
        );
        assert!(
            !html.contains("Ada"),
            "unpaginated list must not load rows, got {html}"
        );
    }

    #[tokio::test]
    async fn unpaginated_table_load_stays_unbounded() {
        // GH #172: the guard lives on the list path (`load_table_page`), not
        // the `None` branch itself — page-owned tables keep loading
        // unbounded through `Table::load` directly.
        use crate::resource::{Table, TableState, TextColumn};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, Clone, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
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
        let cx = CxTestBuilder::new().app_context(db).build();
        let table = Table::<Dummy>::r#for(&cx)
            .id(|d: &Dummy| d.id.to_string())
            .columns(TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                d.name.clone()
            }));
        assert!(table.page_size().is_none());
        let page = table
            .load(
                &cx,
                toasty::stmt::Query::<toasty::stmt::List<Dummy>>::all(),
                &TableState::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            page.rows.len(),
            3,
            "unpaginated tables keep the unbounded branch"
        );
    }

    #[tokio::test]
    async fn list_header_renders_create_entry_point_when_allowed() {
        // GH #162 (Filament's List page `CreateAction` in the page header):
        // the Create link is eager page chrome, gated on `can_create`.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct CreatableResource;
        impl Resource for CreatableResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
                    .paginate(25)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(crate::schema::TextInput::r#for(Dummy::fields().name()))
            }
        }
        struct DenyCreateResource;
        impl Resource for DenyCreateResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                CreatableResource::table(cx)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        async fn list_html<R: Resource>() -> String {
            let db = Db::builder()
                .models(toasty::models!(Dummy))
                .connect("sqlite::memory:")
                .await
                .unwrap();
            db.push_schema().await.unwrap();
            let router = Panel::new("admin")
                .app_context(db)
                .resource::<R>()
                .auth(crate::Auth::disabled())
                .build()
                .expect("panel builds");
            let resp = router
                .handle(
                    http::Request::builder()
                        .uri("/admin/dummies")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert!(resp.status().is_success());
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            String::from_utf8_lossy(&body).to_string()
        }

        let html = list_html::<CreatableResource>().await;
        assert!(
            html.contains("href=\"/admin/dummies/create\"") && html.contains("Create"),
            "allowed list must link to the create page, got {html}"
        );
        let html = list_html::<DenyCreateResource>().await;
        assert!(
            !html.contains("/admin/dummies/create"),
            "denied list must not link to the create page, got {html}"
        );
    }

    #[tokio::test]
    async fn non_editable_resource_hides_edit_links() {
        // GH #162: `editable()` is the `deletable()` (GH #96) counterpart for
        // the per-row Edit link — read-only resources hide it, writable ones
        // link each row to `{list}/{id}/edit`.
        use crate::resource::Resource;
        use http_body_util::BodyExt;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct WritableResource;
        impl Resource for WritableResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn deletable() -> bool {
                false
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
                    .paginate(25)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(crate::schema::TextInput::r#for(Dummy::fields().name()))
            }
        }
        struct LockedResource;
        impl Resource for LockedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn deletable() -> bool {
                false
            }
            fn editable() -> bool {
                false
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                WritableResource::table(cx)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
        }

        async fn list_html<R: Resource>() -> String {
            let mut db = Db::builder()
                .models(toasty::models!(Dummy))
                .connect("sqlite::memory:")
                .await
                .unwrap();
            db.push_schema().await.unwrap();
            toasty::create!(Dummy {
                name: "Ada".to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
            let router = Panel::new("admin")
                .app_context(db)
                .resource::<R>()
                .auth(crate::Auth::disabled())
                .build()
                .expect("panel builds");
            let resp = router
                .handle(
                    http::Request::builder()
                        .uri("/admin/dummies")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert!(resp.status().is_success());
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            String::from_utf8_lossy(&body).to_string()
        }

        let html = list_html::<WritableResource>().await;
        assert!(
            html.contains("/edit") && html.contains(">Edit<"),
            "editable list must link rows to their edit pages, got {html}"
        );
        let html = list_html::<LockedResource>().await;
        assert!(
            !html.contains("/edit") && !html.contains(">Edit<"),
            "non-editable list must not render edit links, got {html}"
        );
    }

    /// The GET `?q=` term is clamped like the shard's (GH #148): bounded
    /// echoed state.
    #[test]
    fn from_cx_clamps_the_search_term() {
        use topcoat::context::CxTestBuilder;

        fn state_for(uri: &str) -> TableState {
            let (parts, ()) = http::Request::builder()
                .uri(uri)
                .body(())
                .unwrap()
                .into_parts();
            let cx = CxTestBuilder::new().request_context(parts).build();
            TableState::from_cx(&cx)
        }

        let long = "x".repeat(500);
        let state = state_for(&format!("/admin/users?q={long}"));
        assert_eq!(
            state.search.as_deref().map(str::len),
            Some(crate::resource::MAX_QUERY_TERM),
            "the GET term is clamped to the same bound as the shard"
        );
        // Blank and absent stay None.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users?q=")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(TableState::from_cx(&cx).search.is_none());
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(TableState::from_cx(&cx).search.is_none());
    }

    #[tokio::test]
    async fn tenant_gated_resource_fails_closed_without_tenant() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct GatedResource;
        impl Resource for GatedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn requires_tenant() -> bool {
                true
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .pk(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
                    .paginate(25)
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(crate::schema::TextInput::r#for(Dummy::fields().name()))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<GatedResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        // No tenant anywhere → 403, not unscoped rows (GH #87).
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        // Valid CSRF but still no tenant → 403 from the tenant gate.
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/create")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("name=Ada&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        // A server-set `Tenant` request extension supplies the tenant → gate
        // passes (create page 200). The header no longer does (GH #131).
        let tenant = uuid::Uuid::new_v4();
        let (mut parts, ()) = http::Request::builder()
            .uri("/admin/dummies/create")
            .body(())
            .unwrap()
            .into_parts();
        parts.extensions.insert(crate::Tenant(tenant));
        let resp = router
            .handle(http::Request::from_parts(parts, Body::empty()))
            .await;
        assert!(
            resp.status().is_success(),
            "tenant-gated GET with tenant must pass the gate, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn streamed_list_renders_error_state_when_load_fails() {
        use topcoat::router::Body;

        #[derive(Debug, toasty::Model)]
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

            fn can_view_any(_cx: &Cx) -> bool {
                true
            }

            fn table(_cx: &Cx) -> Table<Self::Model> {
                // A realistic paginated table: the tampered cursor must reach
                // the decode inside `load_table_page` (only paginated loads
                // decode cursors), not die earlier on missing declarations.
                Table::<Subscriber>::new()
                    .id(|s| s.id.to_string())
                    .pk(|s| s.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Subscriber::fields().email(),
                        |s: &Subscriber| s.email.clone(),
                    ))
                    .paginate(25)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<SubscriberResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        // A tampered `?after=` cursor fails to decode inside the streamed
        // region (GH #79): the page has already streamed with status 200, so
        // the failure must render the branded ErrorState in place — not
        // truncate the stream. This test binary declares no `#[layout]`, so
        // the body is the page fragment stream: page header and toolbar are
        // the "shell still stands" evidence, and the swap payload must be
        // complete (the document-level wrap is proven by the layout tests).
        let response = router
            .handle(
                http::Request::builder()
                    .uri("/admin/subscribers?after=zz-not-a-cursor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            response.status().is_success(),
            "page still streams, got status {}",
            response.status()
        );
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            body.contains(">Subscribers</h1>"),
            "page header must survive the failure: {body}"
        );
        assert!(
            body.contains("Email"),
            "column header must survive the failure: {body}"
        );
        assert!(
            body.contains("Couldn't load Subscribers"),
            "error state must render in the streamed region: {body}"
        );
        // The swap payload arrives complete: topcoat streams swap templates
        // plus the swap script, and a truncated body would cut both.
        assert!(
            body.contains("</template><script>topcoat.swap"),
            "error-state swap payload must be complete: {}",
            &body[body.len().saturating_sub(300)..]
        );
        assert!(
            !body.contains("No records yet"),
            "a failed load is not an empty state: {body}"
        );
        // GH #110: the tampered cursor is the failure itself, so the retry link
        // drops `after`/`before` instead of re-requesting the identical broken
        // URL forever. The rest of the list state still retries.
        assert!(
            body.contains("href=\"/admin/subscribers\""),
            "retry link must target the bare list (cursor dropped): {body}"
        );
        assert!(
            !body.contains("after="),
            "a malformed cursor must not travel into the retry link: {body}"
        );
    }

    #[tokio::test]
    async fn both_cursors_render_error_state_without_cursors() {
        // GH #155: `?after=` + `?before=` together must fail loudly instead of
        // silently preferring `after`. Both tokens below are valid — the old
        // code rendered the `after` page with a 200 and no error.
        use topcoat::router::Body;

        #[derive(Debug, toasty::Model)]
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

            fn can_view_any(_cx: &Cx) -> bool {
                true
            }

            fn table(_cx: &Cx) -> Table<Self::Model> {
                Table::<Subscriber>::new()
                    .id(|s| s.id.to_string())
                    .pk(|s| s.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Subscriber::fields().email(),
                        |s: &Subscriber| s.email.clone(),
                    ))
                    .paginate(1)
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for email in ["a@b.c", "d@e.f"] {
            toasty::create!(Subscriber {
                email: email.to_string()
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = Panel::new("admin")
            .app_context(db.clone())
            .resource::<SubscriberResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        // A valid cursor token: the first page of two rows has a next page.
        let (parts, ()) = http::Request::builder()
            .uri("/admin/subscribers")
            .body(())
            .unwrap()
            .into_parts();
        let cx = topcoat::context::CxTestBuilder::new()
            .request_context(parts)
            .app_context(db)
            .build();
        let table = SubscriberResource::table(&cx);
        let first = load_table_page::<SubscriberResource>(&cx, &table, &TableState::default())
            .await
            .unwrap();
        let cursor = first
            .next_cursor
            .clone()
            .expect("page 1 must have a cursor");

        let response = router
            .handle(
                http::Request::builder()
                    .uri(format!("/admin/subscribers?after={cursor}&before={cursor}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            response.status().is_success(),
            "page still streams, got status {}",
            response.status()
        );
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            body.contains("Couldn't load Subscribers"),
            "conflicting cursors must render the error state, not a page: {body}"
        );
        assert!(
            body.contains("href=\"/admin/subscribers\""),
            "retry link must target the bare list (cursors dropped): {body}"
        );
        assert!(
            !body.contains("after=") && !body.contains("before="),
            "conflicting cursors must not travel into the retry link: {body}"
        );
    }

    #[test]
    fn retry_url_for_error_drops_only_bad_cursors() {
        // GH #110: a malformed cursor can never decode, so its retry link drops
        // pagination; any other failure keeps the full evidence (GH #98).
        let state = TableState {
            search: Some("Ada".to_string()),
            after: Some("cur".to_string()),
            ..TableState::default()
        };
        let bad_cursor = crate::cursor::decode("zz").expect_err("malformed cursor must fail");
        let retry = retry_url_for_error(&state, &bad_cursor, "/admin/users");
        assert!(
            !retry.contains("after="),
            "bad-cursor retry must drop pagination, got {retry}"
        );
        assert!(
            retry.contains("q=Ada"),
            "bad-cursor retry keeps the other state, got {retry}"
        );

        let db_error = topcoat::Error::from(std::io::Error::other("db unavailable"));
        let retry = retry_url_for_error(&state, &db_error, "/admin/users");
        assert!(
            retry.contains("after=cur"),
            "transient failures retry the same evidence, got {retry}"
        );
    }
}
