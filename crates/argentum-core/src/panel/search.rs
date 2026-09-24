//! Live-search registry + shard dispatch (GH #104).
//!
//! `#[shard]` inventory only discovers concrete fns, so each declared
//! resource monomorphizes its table loader here, keyed by list path.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use topcoat::{
    Result,
    context::Cx,
    router::error::forbidden,
    runtime::shard,
    view::{BoxView, View},
};

use super::{
    enforce_auth, enforce_tenant,
    list::{load_table_page, table_error_view, wire_table_actions},
};
use crate::resource::{Resource, TableSignals};

/// One live-table shard invocation (GH #224): the list path the page asks for
/// and the interaction signals it owns.
///
/// The one argument the shard's *handler* takes. The `#[shard]` entry packs
/// its wire parameters into this, and every seam below — the registry lookup,
/// the state rebuild, the load, the render, and the retry link — reads the
/// same value, so a new interaction dimension never changes a signature
/// here.
pub(crate) struct TableSearchArgs {
    /// The list path to rerun: resolved through the registry (an allow-list,
    /// never a raw route).
    pub(crate) path: String,
    /// The page's signals, untrusted by the time the shard reads them back.
    pub(crate) signals: TableSignals,
}

/// A monomorphized live-search table loader, one per declared resource.
///
/// `#[shard]` inventory only discovers concrete fns (GH #104), so the single
/// concrete [`table_search`] shard dispatches through this registry instead
/// of going generic. Built by [`Panel::resource`], keyed by list path.
pub(crate) type SearchFn = Arc<
    dyn for<'a> Fn(
            &'a Cx,
            TableSearchArgs,
        ) -> Pin<Box<dyn Future<Output = Result<BoxView<'a>>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Live-search handlers installed on the app context by [`Panel::build`].
#[derive(Clone, Default)]
pub(crate) struct SearchRegistry(pub(crate) HashMap<String, SearchFn>);

/// Monomorphize `R`'s table loader into a [`SearchFn`]: tenancy + policy gate,
/// then the same load + render the streamed list uses.
///
/// The table catches its own load errors (GH #158): a tampered `after=` /
/// `before=` signal fails to decode inside the shard invocation, and the
/// invocation must render the branded in-region `ErrorState` + retry link
/// (via `super::list::table_error_view`, same as the streamed list) instead of
/// erroring the shard. Auth/tenancy/policy failures still propagate — they
/// are not table evidence.
pub(crate) fn search_handler_for<R: Resource>() -> SearchFn {
    Arc::new(
        |cx: &Cx,
         args: TableSearchArgs|
         -> Pin<Box<dyn Future<Output = Result<BoxView<'_>>> + Send + '_>> {
            Box::pin(async move {
                enforce_auth(cx)?;
                enforce_tenant::<R>(cx)?;
                if !R::can_view_any(cx) {
                    return Err(forbidden().into());
                }
                let table = wire_table_actions::<R>(cx, true);
                let TableSearchArgs { path, signals } = args;
                // One shared bound and one normalization per request (GH
                // #148, GH #206, GH #224): `TableSignals::to_state` applies
                // the same `q` clamp and `filters` bound the GET path applies
                // (GH #205), and the shard `group_by` arg is client input — an
                // unknown value must not echo through the retry link
                // (GH #153). The render below takes the proof and does not
                // normalize again.
                let state = table.normalize_state(&signals.to_state());
                // The retry link inside a failed table writes the same signals
                // the toolbar does (GH #166), so keep a handle for it.
                let retry_signals = signals.clone();
                let rendered = async {
                    let page = load_table_page::<R>(cx, &table, &state).await?;
                    table
                        .render_live_normalized(cx, page, &state, &path, signals)
                        .await
                };
                match rendered.await {
                    Ok(view) => Ok(view),
                    Err(error) => Ok(table_error_view::<R>(
                        cx,
                        &state,
                        &error,
                        &path,
                        Some(&retry_signals),
                    )),
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

/// Live table interactions (GH #104, GH #151): re-renders one resource's table
/// as its signals change, morphing in place per Topcoat #392 (focus, scroll,
/// and typing survive; rows carry stable `id`s from #104 prep).
///
/// The shard owns no state: the page creates the signals ([`TableSignals`]),
/// renders the toolbar against them, and passes their handles here. Search,
/// sort, filters, and pagination all write those signals, so one dependency
/// graph re-renders the table — no navigation, no scroll jump. The swapped
/// region is the table without the search toolbar (the live host owns that
/// slot, so swaps never nest invocations or duplicate inputs).
///
/// Every arg is untrusted shard input: `path` must name a registered list
/// (allow-list, never a raw route), and every signal value is clamped or
/// re-parsed through [`TableSignals::to_state`] like the GET path.
/// Authorization mirrors the list page (`requires_tenant` + `can_view_any`,
/// row scoping via the tenant-scoped query, GH #223); shard POSTs carry no
/// CSRF token, and
/// none is needed for this read-only rerun. The GET toolbar stays as the
/// no-JS fallback.
///
/// The module exists only to carry `allow(too_many_arguments)`: the shard's
/// arity *is* the interaction list — one named parameter per dimension — and
/// that exceeds the lint's default before the macro adds the ambient context.
///
/// The wire stays scalar by choice, not by necessity (GH #224). A struct
/// cannot travel as a shard argument at all (topcoat requires the `expr!`
/// vocabulary, and a struct has no `Surrogated` surrogate the browser's
/// `cx.hydrate` can rebuild), but a *list* can: `Vec<Signal<String>>` and
/// `[Signal<String>; N]` are both vocabulary types that round-trip. Packing
/// the dimensions into one is still refused, because a list couples the
/// browser to this server's ordering — adding or reordering a dimension would
/// silently mismatch the two halves, where a named parameter cannot. So the
/// handler packs its named parameters into [`TableSearchArgs`] for everything
/// *below* the shard, and the wire keeps naming each dimension.
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
        cursor: topcoat::runtime::Signal<String>,
        group_by: topcoat::runtime::Signal<String>,
        bulk: topcoat::runtime::Signal<String>,
    ) -> Result<impl View> {
        let entry = search_entry(cx, &path)?;
        // One argument struct from here down (GH #224): the handler owns the
        // wire arity, nothing below it does.
        entry(
            cx,
            TableSearchArgs {
                path,
                signals: TableSignals {
                    q,
                    filters,
                    sort,
                    dir,
                    cursor,
                    group_by,
                    bulk,
                },
            },
        )
        .await
    }
}
pub(crate) use shard_body::table_search;

#[cfg(test)]
mod tests {
    use toasty::Db;
    use topcoat::router::Body;

    use super::{super::Panel, *};

    /// The live-search shard answers the gate before the registry lookup
    /// (GH #146): an unauthenticated probe cannot distinguish a registered
    /// slug from an unregistered one.
    #[cfg(feature = "auth")]
    #[tokio::test]
    async fn search_shard_answers_auth_before_the_registry_lookup() {
        use topcoat::{context::CxTestBuilder, router::response::IntoResponse};

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model, Clone)]
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
        use std::collections::HashMap;

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model, Clone)]
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
            fn hydrate_form_values(_cx: &Cx, _record: &Dummy) -> HashMap<String, String> {
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

        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        // Positional shard args: q, filters, sort, dir, the single cursor wire
        // (GH #166), group_by, and the bulk handle the table binds its
        // selection transport to.
        let shard_args = |cursor: &str, group_by: &str| {
            format!(
                r#"["/admin/dummies",{}, {}, {}, {}, {}, {}, {}]"#,
                sig(1, ""),
                sig(2, ""),
                sig(3, ""),
                sig(4, ""),
                sig(5, cursor),
                sig(6, group_by),
                sig(7, ""),
            )
        };
        let shard = topcoat::runtime::Shard::id(&table_search);
        let response = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                    .body(Body::from(format!(
                        r#"{{"args":{},"signals":{{}}}}"#,
                        shard_args(&crate::resource::cursor_after("zz-not-a-cursor"), "")
                    )))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "malformed live cursor must render in place, not error the shard"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let table_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            table_html.contains("Couldn't load Dummies"),
            "error state must render in the shard output: {table_html}"
        );
        assert!(
            table_html.contains("role=\"alert\""),
            "error state must carry the alert role: {table_html}"
        );
        assert!(
            table_html.contains("href=\"/admin/dummies\""),
            "retry link must target the bare list (cursor dropped): {table_html}"
        );
        assert!(
            !table_html.contains("after="),
            "a malformed cursor must not travel into the retry link: {table_html}"
        );
        // GH #166: the retry writes the cursor signal in place — the same reset
        // its href spells out — so recovering keeps the signal-held search,
        // filters, and sort instead of reloading the page. The error state
        // renders no other control, so any click binding here is the retry.
        assert!(
            table_html.contains("data-topcoat-on:click"),
            "live retry must write the signals instead of navigating: {table_html}"
        );
        assert!(
            table_html.contains("set((cx.hydrate(&quot;&quot;)).clone())"),
            "live retry must clear the cursor signal: {table_html}"
        );

        // The `before` signal path is symmetric: a tampered backward cursor
        // renders the same cursor-stripped ErrorState. A tampered `group_by`
        // shard arg is normalized with the same state (GH #153), so it must
        // not echo through the retry link either.
        let response = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri(format!("/_topcoat/runtime/shards/{}", shard.as_str()))
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .header(topcoat::router::request::IDENTITY_HEADER, "A".repeat(22))
                    .body(Body::from(format!(
                        r#"{{"args":{},"signals":{{}}}}"#,
                        shard_args(&crate::resource::cursor_before("zz-not-a-cursor"), "nope")
                    )))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "malformed live before-cursor must render in place, not error the shard"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let table_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            table_html.contains("Couldn't load Dummies"),
            "error state must render for a bad before-cursor: {table_html}"
        );
        assert!(
            !table_html.contains("before="),
            "a malformed before-cursor must not travel into the retry link: {table_html}"
        );
        assert!(
            !table_html.contains("group_by"),
            "an unknown group_by must not echo through the shard retry link: {table_html}"
        );
    }

    #[tokio::test]
    async fn live_shard_group_by_signal_drives_grouping() {
        // GH #157: grouping travels as a live signal, not a page-load
        // snapshot — the shard groups by the signal value, so a rerun with
        // the signal set renders headers and a rerun with it cleared does not.
        use std::collections::HashMap;

        use http_body_util::BodyExt;

        use crate::resource::Resource;

        #[derive(Debug, toasty::Model, Clone)]
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
                    .paginate(25)
                    .live_search(true)
            }
            fn hydrate_form_values(_cx: &Cx, _record: &Dummy) -> HashMap<String, String> {
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
            .build()
            .expect("panel builds");

        let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
        let shard_args = |group_by: &str| {
            format!(
                r#"["/admin/dummies",{}, {}, {}, {}, {}, {}, {}]"#,
                sig(1, ""),
                sig(2, ""),
                sig(3, ""),
                sig(4, ""),
                sig(5, ""),
                sig(6, group_by),
                sig(7, ""),
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

        let response = post_shard(shard_args("name")).await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "grouped shard rerun must succeed"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let table_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            table_html.contains("Ada (1 on this page)")
                && table_html.contains("Grace (1 on this page)"),
            "group_by signal must drive group headers in the shard output: {table_html}"
        );

        let response = post_shard(shard_args("")).await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "ungrouped shard rerun must succeed"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let table_html = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            !table_html.contains("on this page"),
            "cleared group_by signal must render no group headers: {table_html}"
        );
    }
}
