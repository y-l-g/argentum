use showcase::app::router_for_tests as router;

mod common;
use common::{body_string, demo_client, seeded_db};

#[tokio::test]
async fn admin_resource_list_page_serve_seeded_users() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;

    let response = client.get("/admin/users").await;

    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let html = body_string(response).await;

    // Layout shell — beautiful: Token classes, sidebar, Token borders.
    // Dark-mode first paint (dark_mode(true)): the document element carries
    // the dark class before any toggle.
    assert!(
        html.contains("<html class=\"dark\">"),
        "missing dark first-paint class in {html}"
    );
    assert!(
        html.contains("border-border") && html.contains("bg-background"),
        "missing admin layout Token chrome in {html}"
    );
    assert!(
        html.contains("data-sidebar=\"sidebar\"") || html.contains("data-sidebar=\"menu\""),
        "missing sidebar in {html}"
    );
    // Sidebar lists curated entries: Team, Writers, Blog Posts, plus the
    // manual Published saved view. No Showcase documentation entry (GH #163).
    assert!(html.contains("Team"), "missing Team label in {html}");
    assert!(
        html.contains("href=\"/admin/users\"") || html.contains("/admin/users"),
        "missing navigation url in {html}"
    );
    assert!(html.contains("Writers"), "missing Writers label in {html}");
    assert!(
        html.contains("href=\"/admin/authors\"") || html.contains("/admin/authors"),
        "missing Writers navigation url in {html}"
    );
    assert!(html.contains("Blog Posts"), "missing Blog Posts label in {html}");
    assert!(
        html.contains("href=\"/admin/posts\"") || html.contains("/admin/posts"),
        "missing Blog Posts navigation url in {html}"
    );
    assert!(html.contains("Published"), "missing manual Published entry in {html}");
    assert!(
        html.contains("/admin/posts?filters=status:published")
            || html.contains("/admin/posts?filters=status%3Apublished")
            || html.contains("status:published"),
        "missing Published saved-view url in {html}"
    );
    assert!(
        !html.contains("href=\"/admin/showcase\""),
        "showcase navigation must be gone in {html}"
    );
    // List page content — production page size (25 per page) shows all
    // seeded users on page 1; cursor pagination across pages is exercised by
    // admin_list_pagination_walks_cursor_links with 23 extra rows.
    assert!(html.contains("Team</h1>"), "missing heading in {html}");
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
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/unknown").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn admin_root_redirects_to_first_resource() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
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
    let router = router(db);
    let client = demo_client(&router).await;
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
    let router = router(db);
    let client = demo_client(&router).await;
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
    let client = demo_client(&router).await;
    // Production page size is 25: seed 23 extra users (3 seeded + 23 = 26)
    // so the list spans two pages. Extra names sort after Grace Hopper.
    {
        let mut db_q = db.clone();
        for i in 0..23 {
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
    let response = client.get("/admin/users").await;
    let page1 = body_string(response).await;

    // Page 1 (name asc, 25 per page): Ada + Alan + Grace, not the last user; a real Next link.
    assert!(page1.contains("Ada Lovelace"), "page1 missing Ada: {page1}");
    assert!(page1.contains("Alan Turing"), "page1 missing Alan: {page1}");
    assert!(page1.contains("Grace Hopper"), "page1 missing Grace: {page1}");
    assert!(
        !page1.contains("User 22"),
        "page1 must not show the last overflow row (page size 25): {page1}"
    );
    let next_href = find_href_with(&page1, "after=")
        .unwrap_or_else(|| panic!("page1 missing Next (after=) link: {page1}"));

    let response = client.get(&next_href).await;
    assert!(
        response.status().is_success(),
        "page2 status {}",
        response.status()
    );
    let page2 = body_string(response).await;
    assert!(
        page2.contains("User 22"),
        "page2 missing overflow row: {page2}"
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

/// Extracts the first `href="…" containing `needle` from an HTML string.
fn find_href_with(html: &str, needle: &str) -> Option<String> {
    let mut rest = html;
    loop {
        let start = rest.find("href=\"")?;
        rest = &rest[start + "href=\"".len()..];
        let end = rest.find('"')?;
        let href = &rest[..end];
        if href.contains(needle) {
            return Some(href.to_string());
        }
        rest = &rest[end..];
    }
}

#[tokio::test]
async fn admin_list_empty_search_shows_no_results_with_clear() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/users?q=zzz-none").await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let html = body_string(response).await;
    assert!(
        html.contains("No prefix matches for"),
        "search-empty state must say No prefix matches: {html}"
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
    let router = router(db);
    let client = demo_client(&router).await;
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
    assert!(
        html.contains("rounded-xl") && html.contains("border-border"),
        "filtered table should still render via Table chrome in {html}"
    );
    assert!(
        !html.contains("Prefix search matches this column"),
        "searchable headers must not carry a loupe, got {html}"
    );
}

#[tokio::test]
async fn users_list_renders_live_search_host_with_get_fallback() {
    // GH #104: the users table opts into the keystroke-live shard; the ?q=
    // GET toolbar stays as the no-JS fallback.
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
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
