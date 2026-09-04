use http_body_util::BodyExt;
use toasty::Db;
use topcoat::router::Body;
use topcoat::view::ViewExt;

use showcase::{
    app::router_for_tests as router,
    models::{User, seed},
};

async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    db
}

#[tokio::test]
async fn admin_resource_list_page_serve_seeded_users() {
    let db = seeded_db().await;
    let router = router(db);

    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);

    // Layout shell — beautiful: Token classes, sidebar, Token borders
    assert!(
        html.contains("border-border") && html.contains("bg-background"),
        "missing admin layout Token chrome in {html}"
    );
    assert!(
        html.contains("data-sidebar=\"sidebar\"") || html.contains("data-sidebar=\"menu\""),
        "missing sidebar in {html}"
    );
    // NavigationItem derived from UserResource and the custom Showcase item.
    assert!(html.contains("Users"), "missing navigation label in {html}");
    assert!(
        html.contains("href=\"/admin/users\"") || html.contains("/admin/users"),
        "missing navigation url in {html}"
    );
    assert!(
        html.contains("href=\"/admin/showcase\""),
        "missing custom Showcase navigation url in {html}"
    );
    // List page content — page 1 of the cursor-paginated list (name asc,
    // 2 per page) shows Ada + Alan; Grace lives on page 2, exercised by
    // admin_list_pagination_walks_cursor_links.
    assert!(
        html.contains("<h1>Users</h1>") || html.contains("Users"),
        "missing heading in {html}"
    );
    assert!(html.contains("Ada Lovelace"), "missing Ada in {html}");
    assert!(html.contains("Alan Turing"), "missing Alan in {html}");
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
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn admin_root_redirects_to_first_resource() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(response.status(), http::StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(http::header::LOCATION).unwrap(),
        "/admin/users"
    );
}

#[tokio::test]
async fn showcase_index_lists_features() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "showcase index status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Showcase"),
        "missing Showcase heading in {html}"
    );
    assert!(
        html.contains("href=\"/admin/showcase\"") && html.contains("aria-current=\"page\""),
        "typed Showcase navigation item should be current on its page: {html}"
    );
    for path in [
        "/admin/showcase/ui",
        "/admin/showcase/dialog",
        "/admin/showcase/schema",
        "/admin/showcase/resource",
        "/admin/showcase/panel",
        "/admin/showcase/table",
        "/admin/showcase/db",
    ] {
        assert!(html.contains(path), "missing link {path} in {html}");
    }
}

#[tokio::test]
async fn showcase_ui_renders_card_and_button_with_tokens() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/ui")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "ui showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Beautiful card") || html.contains("argentum-ui"),
        "missing card title in {html}"
    );
    assert!(
        html.contains("border-border") && html.contains("bg-background"),
        "missing Token border/bg in {html}"
    );
    assert!(
        html.contains("text-muted-foreground"),
        "missing muted text Token in {html}"
    );
    assert!(
        html.contains("shadow-sm") || html.contains("rounded-xl"),
        "missing card shadow/rounded in {html}"
    );
    assert!(html.contains("Primary"), "missing Primary button in {html}");
}

#[tokio::test]
async fn showcase_dialog_renders_notification_and_dialog_with_tokens() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/dialog")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "dialog showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // Notification stack: fixed top-4 right-4, card with border-border bg-background shadow-sm
    assert!(
        html.contains("fixed top-4 right-4") || html.contains("top-4 right-4"),
        "missing notification stack in {html}"
    );
    assert!(
        html.contains("border-border")
            && html.contains("bg-background")
            && html.contains("shadow-sm"),
        "missing notification/dialog card Token in {html}"
    );
    // Dialog: alert_dialog with Primary/Destructive buttons
    assert!(
        html.contains("Delete user?") || html.contains("alert_dialog"),
        "missing dialog title in {html}"
    );
    assert!(
        html.contains("Destructive") || html.contains("Delete"),
        "missing Destructive button in {html}"
    );
    assert!(
        html.contains("Primary") || html.contains("Cancel"),
        "missing Primary/Outline button in {html}"
    );
    // Ensure no ac-* remains in this showcase
    assert!(
        !html.contains("ac-showcase") && !html.contains("ac-"),
        "ac-* should not remain in dialog showcase, got {html}"
    );
}

#[tokio::test]
async fn showcase_schema_renders_variants() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/schema")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "schema showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // Snippets + rendered classes — beautiful: Token classes
    assert!(html.contains("Text::new"), "missing Text snippet in {html}");
    assert!(
        html.contains("text-foreground") || html.contains("text-sm"),
        "missing Text Token in {html}"
    );
    assert!(
        html.contains("rounded-xl") && html.contains("border-border"),
        "missing Section card chrome in {html}"
    );
    assert!(
        html.contains("flex flex-col gap-4"),
        "missing Group Token in {html}"
    );
    assert!(html.contains("grid"), "missing grid in {html}");
    assert!(html.contains("grid-cols-2"), "missing grid cols in {html}");
    // TextInput field — beautiful via label+input Tokens
    assert!(
        html.contains("TextInput::for"),
        "missing TextInput snippet in {html}"
    );
    assert!(
        html.contains("grid gap-1.5"),
        "missing TextInput grid gap-1.5 in {html}"
    );
    assert!(
        html.contains("border-border"),
        "missing input border-border in {html}"
    );
    assert!(html.contains("<input"), "missing input in {html}");
    assert!(
        html.contains("text-sm text-destructive"),
        "missing error slot in {html}"
    );
    // Composition and empty
    assert!(
        html.contains("Schema::empty"),
        "missing empty snippet in {html}"
    );
}

#[tokio::test]
async fn showcase_resource_renders_derives_and_navigation() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "resource showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("BareUserResource"),
        "missing Bare snippet in {html}"
    );
    assert!(
        html.contains("only_ada"),
        "missing only_ada snippet in {html}"
    );
    assert!(
        html.contains("Bare query rows: 3"),
        "missing all count in {html}"
    );
    assert!(
        html.contains("Scoped query rows: 1"),
        "missing scoped count in {html}"
    );
    assert!(
        html.contains("Ada Lovelace"),
        "missing scoped user in {html}"
    );
    assert!(html.contains("Users"), "missing navigation label in {html}");
    assert!(
        html.contains("/admin/bare-users"),
        "missing derived resource URL in {html}"
    );
}

#[tokio::test]
async fn showcase_panel_renders_normalization() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/panel")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "panel showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Panel::new"),
        "missing Panel snippet in {html}"
    );
    assert!(
        html.contains("/admin") && html.contains("/showcase"),
        "missing prefix variants in {html}"
    );
}

#[tokio::test]
async fn showcase_db_renders_memoized_loader() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/db")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "db showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("db(cx)"), "missing db snippet in {html}");
    assert!(
        html.contains("#[memoize"),
        "missing memoize snippet in {html}"
    );
    assert!(
        html.contains("Ada Lovelace") || html.contains("Grace Hopper"),
        "missing user rows in {html}"
    );
}

#[tokio::test]
async fn showcase_table_renders_variants() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/table")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "table showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("TextColumn::for"),
        "missing TextColumn snippet in {html}"
    );
    assert!(
        html.contains("rounded-xl") && html.contains("border-border"),
        "missing table chrome in {html}"
    );
    assert!(
        html.contains("text-muted-foreground"),
        "missing table header Token in {html}"
    );
    assert!(
        html.contains("⌕") || html.contains("search"),
        "missing searchable indicator in {html}"
    );
    assert!(
        html.contains("↕") || html.contains("aria-sort"),
        "missing sortable indicator in {html}"
    );
    assert!(
        html.contains("Ada Lovelace") || html.contains("Name"),
        "missing table rows in {html}"
    );
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
        Some(2),
        "UserResource::table should declare real pagination"
    );
}

#[tokio::test]
async fn admin_list_renders_search_box_and_sort_links() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users?sort=name&dir=asc")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "sorted status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let sorted = String::from_utf8_lossy(&body);
    assert!(
        sorted.contains("sort=name&amp;dir=desc"),
        "missing sort toggle link in {sorted}"
    );
    assert!(
        sorted.contains("aria-sort=\"ascending\""),
        "missing aria-sort=ascending in {sorted}"
    );

    // Sorted descending: direction is declared and the link toggles back.
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users?sort=name&dir=desc")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "desc status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let desc = String::from_utf8_lossy(&body);
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
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let page1 = String::from_utf8_lossy(&body);

    // Page 1 (name asc, 2 per page): Ada + Alan, not Grace; a real Next link.
    assert!(page1.contains("Ada Lovelace"), "page1 missing Ada: {page1}");
    assert!(page1.contains("Alan Turing"), "page1 missing Alan: {page1}");
    assert!(
        !page1.contains("Grace Hopper"),
        "page1 must not show Grace (page size 2): {page1}"
    );
    let next_href = find_href_with(&page1, "after=")
        .unwrap_or_else(|| panic!("page1 missing Next (after=) link: {page1}"));

    let response = router
        .handle(
            http::Request::builder()
                .uri(next_href)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "page2 status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let page2 = String::from_utf8_lossy(&body);
    assert!(
        page2.contains("Grace Hopper"),
        "page2 missing Grace: {page2}"
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
    let response = router
        .handle(
            http::Request::builder()
                .uri(prev_href)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "page1-again status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let page1_again = String::from_utf8_lossy(&body);
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
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users?q=zzz-none")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("No results for"),
        "search-empty state must say No results: {html}"
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
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/users?q=Ada")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "filtered status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
        html.contains("text-muted-foreground") || html.contains("⌕"),
        "filtered table should have searchable indicator in {html}"
    );
}

#[tokio::test]
async fn admin_form_via_resource_renders_text_inputs() {
    use argentum_core::Resource;
    use showcase::app::UserResource;
    use topcoat::context::CxTestBuilder;
    let cx = CxTestBuilder::new().build();
    let form = UserResource::form(&cx);
    let html = form
        .render(&cx)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        html.contains("grid gap-1.5"),
        "Resource::form should render TextInput(s) with grid gap-1.5 in {html}"
    );
    assert!(
        html.contains("border-border") && html.contains("bg-background"),
        "Resource::form should have Token input chrome in {html}"
    );
    assert!(
        html.contains("<input"),
        "Resource::form should contain <input> in {html}"
    );
    assert!(
        html.contains("text-sm text-destructive"),
        "Resource::form should have error slot in {html}"
    );
    assert!(
        html.matches("grid gap-1.5").count() >= 2,
        "Resource::form should have at least 2 fields (name, email) in {html}"
    );
}
