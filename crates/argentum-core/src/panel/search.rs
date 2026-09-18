//! Live-search registry + shard dispatch (GH #104).
//!
//! `#[shard]` inventory only discovers concrete fns, so each declared
//! resource monomorphizes its grid loader here, keyed by list path.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use topcoat::runtime::shard;
use topcoat::{
    Result,
    context::Cx,
    router::error::forbidden,
    view::{BoxView, View},
};

use super::list::{grid_error_view, load_table_page, wire_table_actions};
use super::{enforce_auth, enforce_tenant};
use crate::resource::{Resource, TableState};

/// A monomorphized live-search grid loader, one per declared resource.
///
/// `#[shard]` inventory only discovers concrete fns (GH #104), so the single
/// concrete [`table_search`] shard dispatches through this registry instead
/// of going generic. Built by [`Panel::resource`], keyed by list path.
pub(crate) type SearchFn = Arc<
    dyn for<'a> Fn(
            &'a Cx,
            TableState,
            String,
            crate::resource::TableSignals,
        ) -> Pin<Box<dyn Future<Output = Result<BoxView<'a>>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Live-search handlers installed on the app context by [`Panel::build`].
#[derive(Clone, Default)]
pub(crate) struct SearchRegistry(pub(crate) HashMap<String, SearchFn>);

/// Monomorphize `R`'s grid loader into a [`SearchFn`]: tenancy + policy gate,
/// then the same load + render the streamed list uses.
///
/// The grid catches its own load errors (GH #158): a tampered `after=` /
/// `before=` signal fails to decode inside the shard invocation, and the
/// invocation must render the branded in-region `ErrorState` + retry link
/// (via `super::list::grid_error_view`, same as the streamed list) instead of
/// erroring the shard. Auth/tenancy/policy failures still propagate — they
/// are not grid evidence.
pub(crate) fn search_handler_for<R: Resource>() -> SearchFn {
    Arc::new(
        |cx: &Cx,
         state: TableState,
         path: String,
         signals: crate::resource::TableSignals|
         -> Pin<Box<dyn Future<Output = Result<BoxView<'_>>> + Send + '_>> {
            Box::pin(async move {
                enforce_auth(cx)?;
                enforce_tenant::<R>(cx)?;
                if !R::can_view_any(cx) {
                    return Err(forbidden().into());
                }
                let table = wire_table_actions::<R>(cx, true);
                // Normalize once (GH #153): the shard `group_by` arg is
                // client input — an unknown value must not echo through the
                // retry link. The render re-normalizes internally.
                let state = table.normalize_state(&state);
                let grid = async {
                    let page = load_table_page::<R>(cx, &table, &state).await?;
                    table
                        .render_live_with_state(cx, page, &state, &path, signals)
                        .await
                };
                match grid.await {
                    Ok(view) => Ok(view),
                    Err(error) => Ok(grid_error_view::<R>(cx, &state, &error, &path)),
                }
            })
        },
    )
}

/// Resolve the registered live-search handler for `path`, answering the gate
/// first (GH #146 defense in depth): the registry lookup runs only for an
/// authenticated request, so an unknown `path` cannot be distinguished from a
/// registered one by an unauthenticated probe (404-vs-401 oracle).
fn search_entry(cx: &Cx, path: &str) -> Result<SearchFn> {
    enforce_auth(cx)?;
    topcoat::context::try_app_context::<SearchRegistry>(cx)
        .and_then(|reg| reg.0.get(path).cloned())
        .ok_or_else(|| topcoat::router::error::not_found().into())
}

/// Live table interactions (GH #104, GH #151): re-renders one resource's grid
/// as its signals change, morphing in place per Topcoat #392 (focus, scroll,
/// and typing survive; rows carry stable `id`s from #104 prep).
///
/// The shard owns no state: the page creates the signals ([`TableSignals`]),
/// renders the toolbar against them, and passes their handles here. Search,
/// sort, filters, and pagination all write those signals, so one dependency
/// graph re-renders the grid — no navigation, no scroll jump. The swapped
/// region is the grid without the search toolbar (the live host owns that
/// slot, so swaps never nest invocations or duplicate inputs).
///
/// Every arg is untrusted shard input: `path` must name a registered list
/// (allow-list, never a raw route), and every signal value is clamped or
/// re-parsed through [`TableState::from_live_args`] like the GET path.
/// Authorization mirrors the list page (`requires_tenant` + `can_view_any`,
/// row scoping via `Resource::query`); shard POSTs carry no CSRF token, and
/// none is needed for this read-only rerun. The GET toolbar stays as the
/// no-JS fallback.
///
/// The module exists only to carry `allow(too_many_arguments)`: the shard's
/// arity is its dependency list (one signal per interaction), and the macro
/// expands the handler past the lint's default.
#[allow(clippy::too_many_arguments)]
mod shard_body {
    use super::*;

    #[shard]
    pub(crate) async fn table_search(
        cx: &Cx,
        path: String,
        q: topcoat::runtime::Signal<String>,
        filters: topcoat::runtime::Signal<String>,
        sort: topcoat::runtime::Signal<String>,
        dir: topcoat::runtime::Signal<String>,
        after: topcoat::runtime::Signal<String>,
        before: topcoat::runtime::Signal<String>,
        group_by: topcoat::runtime::Signal<String>,
    ) -> Result<impl View> {
        let entry = search_entry(cx, &path)?;
        let signals = crate::resource::TableSignals {
            q,
            filters,
            sort,
            dir,
            after,
            before,
            group_by,
        };
        // One shared bound (GH #148): the GET `?q=` path and the shard clamp
        // through the same helper, so a term too long for the URL is too long
        // here. Cursors are honored as sent: search/sort/filter handlers clear
        // them when the result set changes, so a live cursor always belongs to
        // the current query.
        let q = crate::resource::clamp_query_term(&signals.q.get());
        let mut state = TableState::from_live_args(
            &q,
            &signals.filters.get(),
            &signals.sort.get(),
            &signals.dir.get(),
            &signals.group_by.get(),
        );
        let after = signals.after.get();
        if !after.trim().is_empty() {
            state.after = Some(after.trim().to_string());
        }
        let before = signals.before.get();
        if !before.trim().is_empty() {
            state.before = Some(before.trim().to_string());
        }
        entry(cx, state, path, signals).await
    }
}
pub(crate) use shard_body::table_search;

#[cfg(test)]
mod tests {
    use super::super::Panel;
    use super::*;
    use toasty::Db;
    use topcoat::router::Body;

    /// The live-search shard answers the gate before the registry lookup
    /// (GH #146): an unauthenticated probe cannot distinguish a registered
    /// slug from an unregistered one.
    #[tokio::test]
    async fn search_shard_answers_auth_before_the_registry_lookup() {
        use topcoat::context::CxTestBuilder;
        use topcoat::router::response::IntoResponse;

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
        }

        // A registry that really knows the `users` slug, so the known-path
        // probe is a resolution the gate must preempt.
        let (parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(crate::auth::RUNTIME_PREFIX)
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::password())
            .app_context(SearchRegistry(HashMap::from([(
                "users".to_string(),
                search_handler_for::<DummyResource>(),
            )])))
            .build();

        // The gate's answer comes before the lookup: both a registered and an
        // unregistered slug answer 401 identically (no 404 oracle).
        let unknown = match search_entry(&cx, "not-a-slug") {
            Ok(_) => panic!("unauthenticated probe must not resolve an entry"),
            Err(err) => err,
        };
        let registered = match search_entry(&cx, "users") {
            Ok(_) => panic!("an unauthenticated probe must never reach the registry"),
            Err(err) => err,
        };
        let unknown_status = unknown
            .into_response(&cx)
            .expect("gate answer renders")
            .status();
        let registered_status = registered
            .into_response(&cx)
            .expect("gate answer renders")
            .status();
        assert_eq!(
            unknown_status, registered_status,
            "unauthenticated probes must not distinguish registered slugs"
        );
        assert_eq!(
            registered_status,
            http::StatusCode::UNAUTHORIZED,
            "runtime probes answer 401 (ADR-0013), got {registered_status}"
        );

        // Auth disabled (the shard's own lookup is what remains): a
        // registered path resolves and an unknown path is a plain 404 again.
        let (parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(crate::auth::RUNTIME_PREFIX)
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new()
            .request_context(parts)
            .app_context(crate::Auth::disabled())
            .app_context(SearchRegistry(HashMap::from([(
                "users".to_string(),
                search_handler_for::<DummyResource>(),
            )])))
            .build();
        assert!(
            search_entry(&cx, "users").is_ok(),
            "with auth disabled a registered path resolves through the lookup"
        );
        let err = match search_entry(&cx, "not-a-slug") {
            Ok(_) => panic!("an unregistered path must not resolve"),
            Err(err) => err,
        };
        assert!(
            err.downcast_ref::<topcoat::router::error::NotFoundError>()
                .is_some(),
            "with auth disabled the unknown path is a plain 404, got {err}"
        );
    }

    #[tokio::test]
    async fn live_shard_malformed_cursor_renders_error_state() {
        // GH #158: a tampered `after=`/`before=` signal fails `cursor::decode`
        // inside the shard invocation — the invocation must render the branded
        // in-region `ErrorState` + retry link (same as the streamed list via
        // `retry_url_for_error`), not error the shard.
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
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<LiveResource>()
            .auth(crate::Auth::disabled())
            .build();

        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        let shard_args = |after: &str, before: &str, group_by: &str| {
            format!(
                r#"["/admin/dummies",{}, {}, {}, {}, {}, {}, {}]"#,
                sig(1, ""),
                sig(2, ""),
                sig(3, ""),
                sig(4, ""),
                sig(5, after),
                sig(6, before),
                sig(7, group_by),
            )
        };
        let shard = topcoat::runtime::Shard::id(&table_search);
        let grid = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                    .body(Body::from(format!(
                        r#"{{"args":{},"signals":{{}}}}"#,
                        shard_args("zz-not-a-cursor", "", "")
                    )))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            grid.status(),
            http::StatusCode::OK,
            "malformed live cursor must render in place, not error the shard"
        );
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            grid_html.contains("Couldn't load Dummies"),
            "error state must render in the shard output: {grid_html}"
        );
        assert!(
            grid_html.contains("role=\"alert\""),
            "error state must carry the alert role: {grid_html}"
        );
        assert!(
            grid_html.contains("href=\"/admin/dummies\""),
            "retry link must target the bare list (cursor dropped): {grid_html}"
        );
        assert!(
            !grid_html.contains("after="),
            "a malformed cursor must not travel into the retry link: {grid_html}"
        );

        // The `before` signal path is symmetric: a tampered backward cursor
        // renders the same cursor-stripped ErrorState. A tampered `group_by`
        // shard arg is normalized with the same state (GH #153), so it must
        // not echo through the retry link either.
        let grid = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                    .body(Body::from(format!(
                        r#"{{"args":{},"signals":{{}}}}"#,
                        shard_args("", "zz-not-a-cursor", "nope")
                    )))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            grid.status(),
            http::StatusCode::OK,
            "malformed live before-cursor must render in place, not error the shard"
        );
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            grid_html.contains("Couldn't load Dummies"),
            "error state must render for a bad before-cursor: {grid_html}"
        );
        assert!(
            !grid_html.contains("before="),
            "a malformed before-cursor must not travel into the retry link: {grid_html}"
        );
        assert!(
            !grid_html.contains("group_by"),
            "an unknown group_by must not echo through the shard retry link: {grid_html}"
        );
    }

    #[tokio::test]
    async fn live_shard_group_by_signal_drives_grouping() {
        // GH #157: grouping travels as a live signal, not a page-load
        // snapshot — the shard groups by the signal value, so a rerun with
        // the signal set renders headers and a rerun with it cleared does not.
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
        struct GroupedResource;
        impl Resource for GroupedResource {
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
                    .group_by("name", |d: &Dummy| d.name.clone())
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
        for name in ["Ada", "Grace"] {
            toasty::create!(Dummy {
                name: name.to_string(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<GroupedResource>()
            .auth(crate::Auth::disabled())
            .build();

        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        let shard_args = |group_by: &str| {
            format!(
                r#"["/admin/dummies",{}, {}, {}, {}, {}, {}, {}]"#,
                sig(1, ""),
                sig(2, ""),
                sig(3, ""),
                sig(4, ""),
                sig(5, ""),
                sig(6, ""),
                sig(7, group_by),
            )
        };
        let post_shard = |args: String| {
            router.handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri(format!(
                        "/_topcoat/runtime/shards/{}",
                        topcoat::runtime::Shard::id(&table_search).as_str()
                    ))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                    .body(Body::from(format!(r#"{{"args":{args},"signals":{{}}}}"#)))
                    .unwrap(),
            )
        };

        let grid = post_shard(shard_args("name")).await;
        assert_eq!(
            grid.status(),
            http::StatusCode::OK,
            "grouped shard rerun must succeed"
        );
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            grid_html.contains("Ada (1 on this page)")
                && grid_html.contains("Grace (1 on this page)"),
            "group_by signal must drive group headers in the shard output: {grid_html}"
        );

        let grid = post_shard(shard_args("")).await;
        assert_eq!(
            grid.status(),
            http::StatusCode::OK,
            "ungrouped shard rerun must succeed"
        );
        let bytes = grid.into_body().collect().await.unwrap().to_bytes();
        let grid_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            !grid_html.contains("on this page"),
            "cleared group_by signal must render no group headers: {grid_html}"
        );
    }
}
