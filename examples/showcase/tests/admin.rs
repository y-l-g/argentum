use showcase::app::router_for_tests as router;

use crate::common::{TestClient, body_string, demo_client, find_href_with, seeded_db, user_count};

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

    // Light first paint (GH #184): a visitor with no stored preference gets a
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
    // Comments. No manual saved view (GH #184) and no Showcase documentation
    // entry (GH #163).
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
    // The create button is worded from the same label (GH #246).
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
    assert_eq!(
        csp(&response).as_deref(),
        Some("frame-ancestors 'self'"),
        "a panel page must carry the directive"
    );
    // Drain the streamed page before the next request: an undrained body keeps
    // the list query's pooled connection, and a later request that resolves the
    // session blocks on the pool.
    let _ = body_string(response).await;

    // A path the router does not match answers 404, hardened all the same.
    let response = client.get("/admin/unknown").await;
    assert_eq!(response.status(), 404);
    assert_eq!(
        csp(&response).as_deref(),
        Some("frame-ancestors 'self'"),
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
    assert_eq!(
        csp(&response).as_deref(),
        Some("frame-ancestors 'self'"),
        "a wrong-method response must carry the directive"
    );

    // The root's temporary redirect to the first resource leaves through the
    // same `Err` branch and keeps the directive.
    let response = client.get("/admin").await;
    assert_eq!(response.status(), http::StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        csp(&response).as_deref(),
        Some("frame-ancestors 'self'"),
        "the root redirect must carry the directive"
    );

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
    assert_eq!(
        csp(&response).as_deref(),
        Some("frame-ancestors 'self'"),
        "the login redirect must carry the directive"
    );
}

#[tokio::test]
async fn admin_root_redirects_to_first_resource() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let response = client.get("/admin").await;

    assert_eq!(response.status(), http::StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(http::header::LOCATION).unwrap(),
        "/admin/users"
    );
}

#[tokio::test]
async fn removed_showcase_routes_are_not_found() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    for path in [
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
    use argentum_core::Resource;
    use showcase::app::UserResource;
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
    assert!(
        page1_again.contains("Ada Lovelace") || page1_again.contains("Alan Turing"),
        "previous page must show page-1 rows: {page1_again}"
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
    .exec(&mut argentum_core::db::db(
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
