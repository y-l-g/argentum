//! Relations on the detail page (item 6).
//!
//! The seed puts every comment on "Hello Toasty" and none on "Second Post",
//! which is the fixture the property needs: the page must show *this* record's
//! related rows — the ones `Resource::query`'s `include` loaded — and not the
//! table's. A database with one post and one comment cannot tell a correct
//! include from a wrong or repeated load.
//!
//! Both posts are selected by title, not by "the one with comments": the
//! showcase's tests share fixtures, and another suite's rows can satisfy a
//! property search like that (the `0…0` tenancy post did exactly that).

use std::sync::Arc;

use showcase::{
    app::router_for_tests as router,
    models::{Comment, Post, REMOVED_COMMENT_BODY},
};

use crate::common::{TestClient, body_string, demo_client, full_db};

/// The commented post and a post with none, by title.
async fn fixture_posts(db: &mut toasty::Db) -> (Post, Post) {
    let commented = Post::all()
        .filter(Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(db)
        .await
        .unwrap()
        .expect("the seed creates Hello Toasty");
    let bare = Post::all()
        .filter(Post::fields().title().eq("Second Post".to_string()))
        .first()
        .exec(db)
        .await
        .unwrap()
        .expect("the seed creates Second Post");
    (commented, bare)
}

#[tokio::test]
async fn detail_page_shows_the_records_own_related_rows() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let (commented, bare) = fixture_posts(&mut db_q).await;

    let related = Comment::all()
        .filter(Comment::fields().post_id().eq(commented.id))
        .exec(&mut db_q)
        .await
        .unwrap();
    assert!(
        !related.is_empty(),
        "the seed attaches comments to Hello Toasty"
    );

    // The commented post's page shows its comments' *rows*. The heading is not
    // asserted: "Comments" is the sidebar nav label present on every panel page
    // and `render_relation`'s own heading is pinned in core
    // (`a_relation_table_renders_every_row_and_column`). The removed
    // placeholder is a related row the policy refuses; its denial has its own
    // test below, so this one stays on the viewable set.
    let html = body_string(client.get(&format!("/admin/posts/{}", commented.id)).await).await;
    for comment in related
        .iter()
        .filter(|comment| comment.body != REMOVED_COMMENT_BODY)
    {
        assert!(
            html.contains(&comment.body),
            "every viewable related row renders: missing {:?} in {html}",
            comment.body
        );
    }

    // …and only its comments. Every seeded comment belongs to the other post,
    // so the bare post's page is the control: a page that rendered the comments
    // *table* instead of the relation would show them here.
    let other = body_string(client.get(&format!("/admin/posts/{}", bare.id)).await).await;
    for comment in Comment::all().exec(&mut db_q).await.unwrap() {
        assert!(
            !other.contains(&comment.body),
            "a post with no comments must not show another post's {:?}",
            comment.body
        );
    }
    assert!(
        other.contains("None."),
        "an empty relation says so instead of rendering an empty table: {other}"
    );
}

/// The relation applies the related resource's `can_view`.
///
/// The reader sees the parent post and the comments that policy admits; the
/// removed placeholder `CommentResource::can_view` refuses does not render. A
/// relation drawn through `render_relation` therefore cannot show a related row
/// the reader may not see, and the row is refused by the resource rather than
/// by a filter the hook could drop.
#[tokio::test]
async fn detail_relation_omits_a_row_the_comment_policy_refuses() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let (commented, _) = fixture_posts(&mut db_q).await;

    let related = Comment::all()
        .filter(Comment::fields().post_id().eq(commented.id))
        .exec(&mut db_q)
        .await
        .unwrap();
    let denied = related
        .iter()
        .find(|comment| comment.body == REMOVED_COMMENT_BODY)
        .expect("the fixture seeds a removed comment on Hello Toasty");
    let visible: Vec<&Comment> = related
        .iter()
        .filter(|comment| comment.body != REMOVED_COMMENT_BODY)
        .collect();
    assert!(
        !visible.is_empty(),
        "the fixture must leave viewable comments to contrast with the denied one"
    );

    let html = body_string(client.get(&format!("/admin/posts/{}", commented.id)).await).await;
    assert!(
        !html.contains(&denied.body),
        "a row the related resource refuses must not render: {html}"
    );
    for comment in visible {
        assert!(
            html.contains(&comment.body),
            "a row the policy admits renders: missing {:?} in {html}",
            comment.body
        );
    }
}

#[tokio::test]
async fn detail_relation_renders_no_list_chrome() {
    // A relation on a record page is a fixed, already-loaded set: the list's
    // pager, search box, bulk column and row actions exist to narrow a query
    // this page never runs.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let (commented, _) = fixture_posts(&mut db_q).await;

    let html = body_string(client.get(&format!("/admin/posts/{}", commented.id)).await).await;
    let body = html
        .split("<h1")
        .nth(1)
        .and_then(|rest| rest.split("</main>").next())
        .expect("the detail page renders inside the shell's main");
    assert!(
        !body.contains("data-row-select"),
        "a record's related rows are not selectable: {body}"
    );
    assert!(
        !body.contains("data-bulk-form"),
        "a record page has no bulk actions: {body}"
    );
    assert!(
        !body.contains("/comments/create"),
        "a record's related rows carry no create chrome: {body}"
    );
}

/// Count the SQL statements one detail page issues.
///
/// The issue asked for the no-N+1 property as either a query count or a fixture
/// where a per-row load would be visible. This is the count: toasty's sqlite
/// driver emits a `driver exec` tracing event per statement, so a subscriber
/// installed for the duration of one request counts them without touching
/// global state or the driver.
async fn statements_for_page(client: &TestClient<'_>, path: &str) -> usize {
    // A thread-local subscriber only sees events emitted on this thread, and
    // the driver runs a statement on whichever thread its connection hands it
    // to — so the first request after the subscriber is installed can slip past
    // it entirely and measure zero while the next one sees the lot (observed:
    // attempt 1 -> 0, attempt 2 -> 7 under a loaded suite). Retrying until the
    // counter is live costs one extra request and removes the flake;
    // `with_rows > 0` below still fails loudly if it never is.
    for _ in 0..8 {
        let count = count_statements_once(client, path).await;
        if count > 0 {
            return count;
        }
    }
    0
}

/// One measurement: install the counting subscriber, fetch the page, count.
async fn count_statements_once(client: &TestClient<'_>, path: &str) -> usize {
    /// toasty's sqlite driver, the layer that actually talks to the database.
    const DRIVER_TARGET: &str = "toasty_driver_sqlite";
    struct OpField(String);
    impl tracing::field::Visit for OpField {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "op" {
                self.0 = format!("{value:?}");
            }
        }
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    use tracing_subscriber::layer::{Layer, SubscriberExt};

    struct SqlCounter(Arc<AtomicUsize>);
    impl<S: tracing::Subscriber> Layer<S> for SqlCounter {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            // Statements only: the driver's own `driver exec`, not the engine's
            // plan/pool chatter, which scales with query *shape* rather than
            // statement count.
            if event.metadata().target() == DRIVER_TARGET {
                let mut op = OpField(String::new());
                event.record(&mut op);
                if op.0.contains("query_sql") || op.0.contains("transaction") {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    let hits = Arc::new(AtomicUsize::new(0));
    // The level filter matters: a subscriber filters by its own max level, and
    // the driver's event is TRACE, so a layer alone would see nothing.
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::TRACE)
        .with(SqlCounter(hits.clone()));
    let _guard = tracing::subscriber::set_default(subscriber);
    // Installing the subscriber does not re-evaluate callsites that other tests
    // in this binary already hit: their interest is cached against the default
    // dispatcher, so the driver's events would be skipped here. Rebuilding the
    // cache makes this thread's subscriber the one those callsites consult.
    tracing::callsite::rebuild_interest_cache();
    // Collect the whole body: a page streams (a skeleton, then the swapped
    // region), and the statements that load the record run while the body is
    // polled. Dropping the response without reading it would measure a page
    // that had not been built yet.
    let _ = body_string(client.get(path).await).await;
    hits.load(Ordering::SeqCst)
}

/// The counting subscriber is thread-local, so the runtime is pinned to the
/// current thread: the default multi-thread runtime can resume a request on a
/// worker that never saw the subscriber. `count_statements_once` rebuilds the
/// callsite interest cache after installing it, because the callsites other
/// tests already hit have cached their interest against the default dispatcher.
#[tokio::test(flavor = "current_thread")]
async fn the_relation_issues_no_query_of_its_own() {
    // The property item 6 asked for, measured rather than argued: a record
    // page's cost must not depend on how many related rows it shows. A per-row
    // load would scale with the comment count; a second query for the relation
    // would show up as a difference between the two pages.
    //
    // The driver runs a statement on whichever thread its connection hands it
    // to, so one measurement can see a subset of a page's statements;
    // a subset is unstable, so the two pages agree only once each measurement
    // has seen its whole page. A per-row load disagrees on every attempt, which
    // is what the assertions below fail on.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let (commented, bare) = fixture_posts(&mut db_q).await;

    let mut observed = (0usize, 0usize);
    for _ in 0..8 {
        observed = (
            statements_for_page(&client, &format!("/admin/posts/{}", commented.id)).await,
            statements_for_page(&client, &format!("/admin/posts/{}", bare.id)).await,
        );
        if observed.0 > 0 && observed.0 == observed.1 {
            break;
        }
    }
    let (with_rows, without_rows) = observed;

    assert!(
        with_rows > 0,
        "the counter must see the page's own statements, or it proves nothing: \
         with_rows={with_rows} without_rows={without_rows}"
    );
    assert_eq!(
        with_rows, without_rows,
        "a record page's cost must not depend on how many related rows it shows: \
         {with_rows} statements with rows, {without_rows} without (GH #187 item 6)"
    );
}

/// A record whose `query` did not include the relation says so.
///
/// The guard the showcase's hook carries, tested by reaching it: load a post
/// through a query that omits the include, then render the hook's view. Without
/// the guard this panics inside `Deferred::get`, which a page render turns into
/// a 500.
#[tokio::test]
async fn a_page_whose_query_skipped_the_include_says_so() {
    use showcase::app::PostResource;
    use tablo_core::Resource;
    use topcoat::{context::CxTestBuilder, view::ViewExt};

    let db = full_db().await;
    let cx = CxTestBuilder::new().app_context(db.clone()).build();
    let mut db_q = db.clone();
    let (commented, _) = fixture_posts(&mut db_q).await;

    // The same post, loaded without `include(comments)`.
    let unloaded = Post::all()
        .filter(Post::fields().id().eq(commented.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post exists");
    assert!(
        unloaded.comments.is_unloaded(),
        "the fixture must load the post without its comments"
    );

    let view = PostResource::view_relations(&cx, &unloaded)
        .expect("the hook answers even when the relation was not loaded");
    let html = view.single().await.unwrap().render(&cx);
    assert!(
        html.contains("were not loaded"),
        "the hook must name the missing include rather than panic: {html}"
    );
}
