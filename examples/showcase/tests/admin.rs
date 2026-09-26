use showcase::app::router_for_tests as router;

use crate::common::{
    TestClient, body_string, demo_client, find_href_with, find_pager_href, row_keys, row_titles,
    seeded_db, user_count,
};

#[tokio::test]
async fn admin_resource_list_page_serve_seeded_users() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let response = client.get("/admin/users").await;

    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let html = body_string(response).await;

    // Light first paint: a visitor with no stored preference gets a
    // light document, and the preference plumbing itself is covered in
    // `auth_check`.
    assert!(
        html.contains("<html>"),
        "showcase must paint light by default in {html}"
    );
    assert!(
        html.contains("data-sidebar=\"sidebar\"") || html.contains("data-sidebar=\"menu\""),
        "missing sidebar in {html}"
    );
    // Sidebar lists one entry per resource: Users, Writers, Blog Posts,
    // Comments. No manual saved view and no Showcase documentation
    // entry.
    assert!(html.contains("Users"), "missing Users label in {html}");
    assert!(
        html.contains("href=\"/admin/users\"") || html.contains("/admin/users"),
        "missing navigation url in {html}"
    );
    assert!(html.contains("Writers"), "missing Writers label in {html}");
    assert!(
        html.contains("href=\"/admin/authors\"") || html.contains("/admin/authors"),
        "missing Writers navigation url in {html}"
    );
    assert!(
        html.contains("Blog Posts"),
        "missing Blog Posts label in {html}"
    );
    assert!(
        html.contains("href=\"/admin/posts\"") || html.contains("/admin/posts"),
        "missing Blog Posts navigation url in {html}"
    );
    assert!(
        html.contains("Comments"),
        "missing Comments label in {html}"
    );
    assert!(
        html.contains("href=\"/admin/comments\"") || html.contains("/admin/comments"),
        "missing Comments navigation url in {html}"
    );
    // GH #184: no Published saved view — it would duplicate the Blog Posts
    // table with a filter, and it is the only arrangement that would highlight
    // two sidebar entries at once.
    assert!(
        !html.contains("status:published"),
        "the redundant Published saved view must be gone: {html}"
    );
    assert!(
        !html.contains("href=\"/admin/showcase\""),
        "showcase navigation must be gone in {html}"
    );
    // List page content — production page size (25 per page) shows all
    // seeded users on page 1; cursor pagination across pages is exercised by
    // admin_list_pagination_walks_cursor_links, which seeds one row past the
    // page size.
    assert!(html.contains("Users</h1>"), "missing heading in {html}");
    // The create button is worded from the same label.
    assert!(
        html.contains("Create Users"),
        "missing create entry point in {html}"
    );
    assert!(html.contains("Ada Lovelace"), "missing Ada in {html}");
    assert!(html.contains("Alan Turing"), "missing Alan in {html}");
    assert!(html.contains("Grace Hopper"), "missing Grace in {html}");
    assert!(
        html.contains("ada@example.com"),
        "missing Ada email in {html}"
    );
    assert!(
        html.contains("alan@example.com"),
        "missing Alan email in {html}"
    );
}

#[tokio::test]
async fn admin_unknown_route_is_not_found() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin/unknown").await;
    assert_eq!(response.status(), 404);
}

/// GH #295: the `frame-ancestors` layer covers every response the panel's layer
/// chain produces — the 404 for an unmatched path, the 405 for a wrong method,
/// and the redirects the handlers build as errors — not only the 200 pages.
#[tokio::test]
async fn error_responses_carry_frame_ancestors() {
    use topcoat::router::Body;

    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csp = |response: &http::Response<Body>| {
        response
            .headers()
            .get(http::header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };

    // The success path keeps the directive.
    let response = client.get("/admin/users").await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    assert!(
        csp(&response).is_some_and(|policy| policy.contains("frame-ancestors")),
        "a panel page must carry the directive"
    );
    // Drain the streamed page before the next request: an undrained body keeps
    // the list query's pooled connection, and a later request that resolves the
    // session blocks on the pool.
    let _ = body_string(response).await;

    // A path the router does not match answers 404, hardened all the same.
    let response = client.get("/admin/unknown").await;
    assert_eq!(response.status(), 404);
    assert!(
        csp(&response).is_some_and(|policy| policy.contains("frame-ancestors")),
        "an unmatched route must carry the directive"
    );

    // A method the login route does not accept answers 405. The login path
    // bypasses the auth gate, so no session is needed to reach the route table.
    let request = http::Request::builder()
        .method(http::Method::PATCH)
        .uri("/admin/login")
        .body(Body::empty())
        .unwrap();
    let response = router.handle(request).await;
    assert_eq!(response.status(), 405, "PATCH on the login route is a 405");
    assert!(
        csp(&response).is_some_and(|policy| policy.contains("frame-ancestors")),
        "a wrong-method response must carry the directive"
    );

    // The dashboard at the panel root renders like any other page and keeps
    // the directive.
    let response = client.get("/admin").await;
    assert!(
        response.status().is_success(),
        "GET /admin serves the dashboard, got {}",
        response.status()
    );
    assert!(
        csp(&response).is_some_and(|policy| policy.contains("frame-ancestors")),
        "the dashboard must carry the directive"
    );
    let _ = body_string(response).await;

    // The gate's login redirect does too: an unauthenticated page request is
    // answered by a redirect to the login route.
    let anonymous = TestClient::new(&router);
    let response = anonymous.get("/admin/users").await;
    assert_eq!(
        response.status(),
        http::StatusCode::TEMPORARY_REDIRECT,
        "an unauthenticated page request redirects to login"
    );
    assert!(
        response
            .headers()
            .get(http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with("/admin/login")),
        "the redirect must name the login route"
    );
    assert!(
        csp(&response).is_some_and(|policy| policy.contains("frame-ancestors")),
        "the login redirect must carry the directive"
    );
}

#[tokio::test]
async fn admin_root_serves_the_dashboard() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin").await;

    assert!(
        response.status().is_success(),
        "GET /admin serves the dashboard, got {}",
        response.status()
    );
    let html = body_string(response).await;
    assert!(
        html.contains(">Dashboard</h1>"),
        "the root renders the dashboard heading: {html}"
    );
    assert!(
        html.contains("data-live-feed"),
        "the dashboard carries the live feed: {html}"
    );
}

#[tokio::test]
async fn panel_chrome_lists_dashboard_media_and_blog_link() {
    // The shell's own chrome, pinned on a resource page so the dashboard's
    // assertions above stay on the root: Dashboard first, the media library
    // entry after the resources, and the public blog link in the header.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let html = body_string(client.get("/admin/users").await).await;

    let dashboard_at = html
        .find("href=\"/admin\"")
        .unwrap_or_else(|| panic!("missing the Dashboard entry in {html}"));
    for label in ["Users", "Writers", "Blog Posts", "Comments"] {
        let at = html
            .find(label)
            .unwrap_or_else(|| panic!("missing {label} in {html}"));
        assert!(
            dashboard_at < at,
            "the Dashboard entry must sort first, before {label}: {html}"
        );
    }
    assert!(
        html.contains("href=\"/admin/media\"") && html.contains("Media library"),
        "missing the media library entry in {html}"
    );
    let media_at = html.find("href=\"/admin/media\"").unwrap();
    let comments_at = html
        .find("href=\"/admin/comments\"")
        .expect("the Comments entry");
    assert!(
        comments_at < media_at,
        "the media library sorts after the resources: {html}"
    );
    assert!(
        html.contains("href=\"/blog\"") && html.contains(">View blog<"),
        "missing the header blog link in {html}"
    );
    // Same tab: the blog link carries no target.
    let blog_at = html.find("href=\"/blog\"").expect("the blog link");
    let tag_start = html[..blog_at].rfind("<a").expect("its opening tag");
    let tag_end = html[tag_start..].find('>').expect("the tag's end") + tag_start;
    assert!(
        !html[tag_start..tag_end].contains("target="),
        "the blog link stays in the same tab: {html}"
    );
    // Active state: on a resource page the resource is current and the
    // exact-matched Dashboard is not — one highlight, not two.
    assert_eq!(
        html.matches("data-active=\"true\"").count(),
        1,
        "exactly one sidebar entry is current on a resource page: {html}"
    );
}

#[tokio::test]
async fn dashboard_marks_only_itself_current() {
    // The mirror half: on the root the Dashboard entry is the current one,
    // and no resource entry highlights alongside it.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let html = body_string(client.get("/admin").await).await;
    assert_eq!(
        html.matches("data-active=\"true\"").count(),
        1,
        "exactly one sidebar entry is current on the dashboard: {html}"
    );
    assert!(
        html.contains("aria-current=\"page\""),
        "the current entry names itself: {html}"
    );
}

#[tokio::test]
async fn removed_showcase_routes_are_not_found() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    for path in [
        "/admin/live",
        "/admin/showcase",
        "/admin/showcase/ui",
        "/admin/showcase/dialog",
        "/admin/showcase/panel",
        "/admin/showcase/resource",
        "/admin/showcase/schema",
        "/admin/showcase/table",
        "/admin/showcase/db",
    ] {
        let response = client.get(path).await;
        assert_eq!(response.status(), 404, "{path} should be gone");
    }
}

#[tokio::test]
async fn admin_table_via_resource_has_searchable_sortable() {
    use showcase::app::UserResource;
    use tablo_core::Resource;
    use topcoat::context::CxTestBuilder;
    let cx = CxTestBuilder::new().build();
    let table = UserResource::table(&cx);
    assert!(
        table.search_expr("Ada").is_some(),
        "searchable column should produce expr"
    );
    assert!(
        table.order_by(false).is_some(),
        "sortable column should produce order_by"
    );
    assert_eq!(
        table.page_size(),
        Some(25),
        "UserResource::table should declare production pagination"
    );
}

#[tokio::test]
async fn admin_list_renders_search_box_and_sort_links() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin/users").await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let html = body_string(response).await;
    // Real search UI — a GET form with a q input, not a URL-only affordance.
    assert!(
        html.contains("<form") && html.contains("name=\"q\""),
        "missing search form in {html}"
    );
    assert!(
        html.contains("type=\"search\""),
        "missing search input type in {html}"
    );
    // Real sort controls — links that drive ?sort/&dir, aria-sort present.
    // Unsorted page: the first click sorts ascending; the active column
    // carries aria-sort="none".
    assert!(
        html.contains("sort=name&amp;dir=asc"),
        "missing sort link in {html}"
    );
    assert!(
        html.contains("aria-sort=\"none\""),
        "missing aria-sort on sortable column in {html}"
    );

    // Sorted ascending: the same link toggles to descending and the column
    // declares aria-sort="ascending".
    let response = client.get("/admin/users?sort=name&dir=asc").await;
    assert!(
        response.status().is_success(),
        "sorted status {}",
        response.status()
    );
    let sorted = body_string(response).await;
    assert!(
        sorted.contains("sort=name&amp;dir=desc"),
        "missing sort toggle link in {sorted}"
    );
    assert!(
        sorted.contains("aria-sort=\"ascending\""),
        "missing aria-sort=ascending in {sorted}"
    );

    // Sorted descending: direction is declared and the link toggles back.
    let response = client.get("/admin/users?sort=name&dir=desc").await;
    assert!(
        response.status().is_success(),
        "desc status {}",
        response.status()
    );
    let desc = body_string(response).await;
    assert!(
        desc.contains("aria-sort=\"descending\""),
        "missing aria-sort=descending in {desc}"
    );
    assert!(
        desc.contains("sort=name&amp;dir=asc"),
        "missing toggle back to ascending in {desc}"
    );
}

#[tokio::test]
async fn admin_list_pagination_walks_cursor_links() {
    use showcase::models::User;

    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // GH #217: derive the overflow from the fixture and the page size rather
    // than seeding a literal 23. Production page size is 25, so page 1 holds
    // the seeded roster plus `extra - 1` filler rows and exactly one filler row
    // spills to page 2.
    let seeded = user_count(&db).await;
    let page_size = 25usize;
    let extra = page_size - seeded + 1;
    let last = format!("User {:02}", extra - 1);
    {
        let mut db_q = db.clone();
        for i in 0..extra {
            let name = format!("User {:02}", i);
            let email = format!("user{:02}@example.com", i);
            toasty::create!(User {
                name: name,
                email: email,
                role: "member",
                active: true,
                created_at: "2024-03-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db_q)
            .await
            .unwrap();
        }
    }
    // Sorted ascending so the cursor links carry two query parameters: the
    // pager must preserve that state, which is also what makes the `&amp;`
    // decoding below observable (a one-parameter URL has no `&` to encode).
    let response = client.get("/admin/users?sort=name&dir=asc").await;
    let page1 = body_string(response).await;
    let page1_titles = row_titles(&page1);

    // Page 1 (name asc, 25 per page): Ada + Alan + Grace, not the last user; a real Next link.
    assert!(page1.contains("Ada Lovelace"), "page1 missing Ada: {page1}");
    assert!(page1.contains("Alan Turing"), "page1 missing Alan: {page1}");
    assert!(
        page1.contains("Grace Hopper"),
        "page1 missing Grace: {page1}"
    );
    assert!(
        !page1.contains(&last),
        "page1 must not show the last overflow row {last} (page size {page_size}): {page1}"
    );
    let next_href = find_href_with(&page1, "after=")
        .unwrap_or_else(|| panic!("page1 missing Next (after=) link: {page1}"));
    // GH #217: the href is followed as a request URI, so it must be the URL a
    // browser would send — decoded, never `&amp;`.
    assert!(
        !next_href.contains("&amp;"),
        "the Next link must be followed decoded, got {next_href}"
    );
    assert!(
        next_href.contains("sort=name") && next_href.contains("dir=asc"),
        "the pager must preserve the sort state, got {next_href}"
    );

    let response = client.get(&next_href).await;
    assert!(
        response.status().is_success(),
        "page2 status {}",
        response.status()
    );
    let page2 = body_string(response).await;
    assert!(
        page2.contains(&last),
        "page2 missing the overflow row {last}: {page2}"
    );
    assert!(
        !page2.contains("Ada Lovelace") && !page2.contains("Alan Turing"),
        "page2 must not repeat page 1 rows: {page2}"
    );
    assert!(
        find_href_with(&page2, "before=").is_some(),
        "page2 missing Previous (before=) link: {page2}"
    );

    // Following Previous returns to the first page.
    let prev_href = find_href_with(&page2, "before=").unwrap();
    assert!(
        !prev_href.contains("&amp;"),
        "the Previous link must be followed decoded, got {prev_href}"
    );
    let response = client.get(&prev_href).await;
    assert!(
        response.status().is_success(),
        "page1-again status {}",
        response.status()
    );
    let page1_again = body_string(response).await;
    // The same rows, in the same order: a one-row page, a repeated page 2, or
    // a page that merely contains a seeded name would all pass a looser check.
    assert_eq!(
        row_titles(&page1_again),
        page1_titles,
        "following Previous must restore page 1 unchanged"
    );
}

/// The pager walks a descending ordering without skipping or repeating rows.
///
/// The cursor carries the ordering's sort values; the direction lives in
/// `?dir=desc` and the query the loader builds from it. A page boundary that
/// compared the cursor against the wrong ordering would drop the rest of the
/// result set or serve page-1 rows again, and the row order would stop being
/// descending.
#[tokio::test]
async fn admin_list_pagination_walks_descending_cursor_links() {
    use showcase::models::User;

    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let seeded = user_count(&db).await;
    let page_size = 25usize;
    // One row past a full page, so the walk spans exactly two pages.
    let extra = page_size - seeded + 1;
    {
        let mut db_q = db.clone();
        for i in 0..extra {
            toasty::create!(User {
                name: format!("User {:02}", i),
                email: format!("desc{:02}@example.com", i),
                role: "member",
                active: true,
                created_at: "2024-03-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db_q)
            .await
            .unwrap();
        }
    }
    let total = user_count(&db).await;
    assert!(total > page_size, "the fixture must span two pages");

    let mut url = "/admin/users?sort=name&dir=desc".to_string();
    let mut names = Vec::new();
    let mut keys = Vec::new();
    for _ in 0..8 {
        let response = client.get(&url).await;
        assert!(response.status().is_success(), "GET {url}");
        let html = body_string(response).await;
        assert!(
            html.contains("sort=name") && html.contains("dir=desc"),
            "the pager must keep the descending state, got {html}"
        );
        names.extend(row_titles(&html));
        keys.extend(row_keys(&html));
        match find_pager_href(&html, "after=") {
            Some(next) => {
                assert!(!next.contains("&amp;"), "the link must be decoded: {next}");
                url = next;
            }
            None => break,
        }
    }
    assert_eq!(names.len(), total, "the walk must cover every row");
    let mut descending = names.clone();
    descending.sort();
    descending.reverse();
    assert_eq!(names, descending, "the pages must stay in descending order");
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    // One row short of the total: the SSO-denied row renders no checkbox, so
    // its key never reaches the page. Its single appearance is covered by the
    // title walk above.
    assert_eq!(
        unique.len(),
        total - 1,
        "no row may appear on two pages of a descending walk"
    );
}

/// The pager's cursor keeps its tie-breaker when the sort value repeats.
///
/// Every tied row has the same sort value, so only the primary key separates
/// them: a boundary that compared the sort value alone would stop at the first
/// tied row (dropping the rest) or serve the tie twice. The tied group spans
/// the page boundary, so the walk must split it and still cover every row.
#[tokio::test]
async fn admin_list_pagination_keeps_tied_sort_values() {
    use showcase::models::User;

    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let seeded = user_count(&db).await;
    let page_size = 25usize;
    let tied = 30usize;
    {
        let mut db_q = db.clone();
        for i in 0..tied {
            toasty::create!(User {
                name: "Tied".to_string(),
                email: format!("tied{:02}@example.com", i),
                role: "member",
                active: true,
                created_at: "2024-03-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db_q)
            .await
            .unwrap();
        }
    }
    let total = user_count(&db).await;

    let page1 = body_string(client.get("/admin/users?sort=name&dir=asc").await).await;
    let page1_titles = row_titles(&page1);
    assert_eq!(page1_titles.len(), page_size, "page 1 must be full");
    // Every seeded name sorts before "Tied", so the tie fills the tail of
    // page 1 and all of page 2.
    assert_eq!(
        page1_titles.iter().filter(|title| *title == "Tied").count(),
        page_size - seeded,
        "the tied group must start inside page 1: {page1}"
    );
    let next = find_pager_href(&page1, "after=").expect("page 2 link");
    let page2 = body_string(client.get(&next).await).await;
    let page2_titles = row_titles(&page2);
    assert_eq!(
        page2_titles.len(),
        total - page_size,
        "page 2 holds the rest"
    );
    assert!(
        page2_titles.iter().all(|title| title == "Tied"),
        "page 2 must continue the tied group: {page2}"
    );

    // The tie-breaker is what makes the split exact: without it the second
    // page would repeat the tie or drop it, so the keys would not cover the
    // selectable rows. One row short of the total: the SSO-denied row renders
    // no checkbox, so its key never reaches the page; its title is covered by
    // the page-1 assertions above.
    let mut keys = row_keys(&page1);
    keys.extend(row_keys(&page2));
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        total - 1,
        "the tie-breaker must not skip or repeat a row"
    );
}

/// GH #116: search matches anywhere in the value, not just a prefix, and the
/// term is escaped — a literal `%` matches that character instead of acting as
/// a wildcard (which would have matched every row).
#[tokio::test]
async fn admin_list_search_matches_substrings_and_escapes_wildcards() {
    let db = seeded_db().await;
    let router = router(db.clone());

    // Mid-string term: "vela" sits inside "Ada Lovelace" -> 1 row.
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin/users?q=vela").await;
    assert!(response.status().is_success());
    let html = body_string(response).await;
    assert!(
        html.contains("Ada Lovelace"),
        "a mid-string term must match: {html}"
    );
    assert!(
        !html.contains("Alan Turing"),
        "a mid-string term must not match the other rows: {html}"
    );

    // A user whose value contains a literal percent sign.
    toasty::create!(showcase::models::User {
        name: "100% Ada".to_string(),
        email: "percent@example.com".to_string(),
        role: "admin".to_string(),
        active: true,
        created_at: "2024-02-01T09:30:00Z"
            .parse::<jiff::Timestamp>()
            .expect("timestamp"),
    })
    .exec(&mut tablo_core::db::db(
        &topcoat::context::CxTestBuilder::new()
            .app_context(db.clone())
            .build(),
    ))
    .await
    .expect("seed the percent user");

    // The same client: the new row is visible through the session it already
    // holds.
    let response = client.get("/admin/users?q=100%25").await;
    let html = body_string(response).await;
    assert!(
        html.contains("100% Ada"),
        "an escaped literal percent must match its row: {html}"
    );
    assert!(
        !html.contains("Ada Lovelace") && !html.contains("Grace Hopper"),
        "an escaped percent must not act as a wildcard: {html}"
    );
}

#[tokio::test]
async fn admin_list_empty_search_shows_no_results_with_clear() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin/users?q=zzz-none").await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let html = body_string(response).await;
    assert!(
        html.contains("No matches for"),
        "search-empty state must say No matches: {html}"
    );
    assert!(
        !html.contains("No records yet"),
        "search-empty state must not claim no records: {html}"
    );
    assert!(
        !html.contains("Create record"),
        "dead Create button must stay gone: {html}"
    );
    assert!(
        html.contains("Clear search"),
        "missing Clear search link: {html}"
    );
}

#[tokio::test]
async fn admin_list_filters_via_q_param() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin/users?q=Ada").await;
    assert!(
        response.status().is_success(),
        "filtered status {}",
        response.status()
    );
    let html = body_string(response).await;
    assert!(
        html.contains("Ada Lovelace"),
        "filtered should contain Ada in {html}"
    );
    assert!(
        !html.contains("Grace Hopper"),
        "filtered should not contain Grace in {html}"
    );
}

#[tokio::test]
async fn users_list_renders_live_search_host_with_get_fallback() {
    // GH #104: the users table opts into the keystroke-live shard; the ?q=
    // GET toolbar stays as the no-JS fallback.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/users").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("data-live-search"),
        "users list must render the live host, got {html}"
    );
    assert!(
        html.contains("<noscript>"),
        "live list must keep the GET fallback, got {html}"
    );
    // Seeded rows still stream in beneath the host.
    assert!(html.contains("Ada Lovelace"), "missing Ada in {html}");
}
