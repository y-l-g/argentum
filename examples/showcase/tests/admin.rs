use topcoat::view::ViewExt;

use showcase::app::router_for_tests as router;

mod common;
use common::{body_string, demo_client, form_body, input_value, response_cookies, seeded_db};

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
    assert!(html.contains("Users</h1>"), "missing heading in {html}");
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
async fn showcase_index_lists_features() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase").await;
    assert!(
        response.status().is_success(),
        "showcase index status {}",
        response.status()
    );
    let html = body_string(response).await;
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
    // The feature list is a table (GH #151 §4): headers plus a description
    // cell, not the old `<ul>`.
    assert!(
        html.contains(">Page</th>") && html.contains(">What it shows</th>"),
        "feature list should render as a table with headers: {html}"
    );
    assert!(
        html.contains("per-request memoization"),
        "feature table should carry the page descriptions: {html}"
    );
}

#[tokio::test]
async fn showcase_ui_renders_every_component_family() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/ui").await;
    assert!(
        response.status().is_success(),
        "ui showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
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
    // Every component family is on the page: native controls, disclosure,
    // loading shapes, and the composite previews (GH #151 §5). The needles
    // are page-specific IDs/markup so shell chrome cannot satisfy them.
    for needle in [
        "id=\"ui-email\"",
        "id=\"ui-region\"",
        "id=\"ui-terms\"",
        "id=\"ui-airplane\"",
        "id=\"ui-weekly\"",
        "id=\"ui-tabs\"",
        "name=\"ui-faq\"",
        "<progress",
        "<textarea",
        "<select",
        "role=\"radiogroup\"",
        "Couldn't load Users</p>",
        "Token-only customization",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
}

/// The tabs demo is server state, not a dead control: `?tab=` picks the
/// panel and the active trigger is marked current (GH #151 §5).
#[tokio::test]
async fn showcase_ui_tabs_reflect_the_url() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let default = client.get("/admin/showcase/ui").await;
    let default_html = body_string(default).await;
    assert!(
        default_html.contains("Overview panel — rendered from ?tab=overview (the default)."),
        "the overview panel is the default: {default_html}"
    );
    let response = client.get("/admin/showcase/ui?tab=activity").await;
    assert!(response.status().is_success());
    let html = body_string(response).await;
    assert!(
        html.contains("Activity panel — rendered from ?tab=activity."),
        "the activity tab should be the rendered panel: {html}"
    );
    assert!(
        !html.contains("Overview panel — rendered from ?tab=overview"),
        "only one tab panel should render: {html}"
    );
    assert!(
        html.contains("href=\"?tab=activity#ui-tabs\"") && html.contains("aria-current=\"page\""),
        "the active trigger should be marked current and land back on the demo: {html}"
    );
}

#[tokio::test]
async fn showcase_dialog_renders_notification_and_dialog_with_tokens() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/dialog").await;
    assert!(
        response.status().is_success(),
        "dialog showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
    // Toast stack: the shadcn/Sonner surface, fixed bottom-right.
    assert!(
        html.contains("data-sonner-toaster") && html.contains("bottom-4"),
        "missing toast stack in {html}"
    );
    assert!(
        html.contains("aria-live=\"polite\""),
        "missing polite live region in {html}"
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
    // Ensure no ac-* remains in this showcase. Match the class attribute
    // (not bare "ac-"), because the CSRF token is a random UUID that can
    // contain the same substring and flake the assertion.
    assert!(
        !html.contains("ac-showcase") && !html.contains("class=\"ac-"),
        "ac-* should not remain in dialog showcase, got {html}"
    );
}

/// The dialog page's toast demo is a real POST: it verifies CSRF, flashes a
/// Notification, redirects (PRG), and the next GET renders the toast in the
/// shell's toaster (GH #151 §6).
#[tokio::test]
async fn showcase_dialog_toast_demo_flashes_a_real_notification() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let page = client.get("/admin/showcase/dialog").await;
    let cookies = response_cookies(&page);
    let html = body_string(page).await;
    assert!(
        !html.contains("data-type="),
        "no toast should be present before the demo runs: {html}"
    );
    let csrf = input_value(&html, "csrf_token")
        .unwrap_or_else(|| panic!("toast demo must embed a csrf_token input: {html}"));
    for (status, title) in [
        ("success", "User created"),
        ("info", "Heads up"),
        ("warning", "Careful"),
        ("error", "Something failed"),
    ] {
        let posted = client
            .cookies(&cookies)
            .post_form(
                "/admin/showcase/dialog/notify",
                form_body(&[("csrf_token", &csrf), ("status", status)]),
            )
            .await;
        assert_eq!(posted.status(), 303, "notify redirects (POST/Redirect/Get)");
        assert_eq!(
            posted.headers().get("location").unwrap(),
            "/admin/showcase/dialog",
            "the demo lands back on the dialog page"
        );
        let flash = response_cookies(&posted);
        assert!(
            flash
                .iter()
                .any(|(name, _)| name == "__Host-argentum_notification"),
            "the notification must ride the flash cookie: {flash:?}"
        );
        let followed = client
            .cookies(&cookies)
            .cookies(&flash)
            .get("/admin/showcase/dialog")
            .await;
        let html = body_string(followed).await;
        assert!(
            html.contains(&format!("data-type=\"{status}\""))
                && html.contains(&format!("\">{title}</div>")),
            "the flashed notification should render as a {status} toast: {html}"
        );
    }
}

#[tokio::test]
async fn showcase_schema_renders_variants() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/schema").await;
    assert!(
        response.status().is_success(),
        "schema showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
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
        html.contains("@container/field-group"),
        "missing field_group Token in {html}"
    );
    assert!(html.contains("grid"), "missing grid in {html}");
    assert!(html.contains("grid-cols-2"), "missing grid cols in {html}");
    // TextInput field — beautiful via label+input Tokens
    assert!(
        html.contains("TextInput::for"),
        "missing TextInput snippet in {html}"
    );
    assert!(
        html.contains("data-slot=\"field\""),
        "missing TextInput field wrapper in {html}"
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
    // Required is inferred, not called: exactly the four non-optional
    // fields (two demos + two form fields) carry `aria-required`, and the
    // `.optional()` demos carry none.
    assert_eq!(
        html.matches("aria-required=\"true\"").count(),
        4,
        "required should be inferred for the non-optional fields only: {html}"
    );
    // The live-validation form is a real POST with a CSRF token, and
    // `novalidate` lets the server errors render in the browser.
    assert!(
        html.contains("action=\"/admin/showcase/schema\"")
            && html.contains("csrf_token")
            && html.contains("novalidate")
            && html.contains("Live validation"),
        "missing validation form in {html}"
    );
}

/// §8's validation demo is a real form: an empty required field and an
/// invalid email come back inline, and a valid submit redirects with a toast.
#[tokio::test]
async fn showcase_schema_validation_form_reports_errors_and_flashes_success() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let page = client.get("/admin/showcase/schema").await;
    let cookies = response_cookies(&page);
    let html = body_string(page).await;
    let csrf = input_value(&html, "csrf_token")
        .unwrap_or_else(|| panic!("validation form must embed a csrf_token input: {html}"));
    let invalid = client
        .cookies(&cookies)
        .post_form(
            "/admin/showcase/schema",
            form_body(&[
                ("csrf_token", &csrf),
                ("name", ""),
                ("email", "not-an-email"),
            ]),
        )
        .await;
    assert_eq!(invalid.status(), 200, "invalid submits re-render the page");
    let html = body_string(invalid).await;
    assert!(
        html.contains("Name is required") && html.contains("Email must be a valid email"),
        "inline errors should render: {html}"
    );
    assert!(
        html.contains("value=\"not-an-email\"") && html.contains("aria-invalid=\"true\""),
        "invalid submits should preserve values and mark the field: {html}"
    );
    assert!(
        html.contains("type=\"email\""),
        "the email field should render as an email input: {html}"
    );
    // CSRF stays fail-closed on the demo form too.
    let no_csrf = client
        .cookies(&cookies)
        .post_form(
            "/admin/showcase/schema",
            form_body(&[("name", "Ada"), ("email", "ada@example.com")]),
        )
        .await;
    assert_eq!(no_csrf.status(), 403, "a missing CSRF token is forbidden");
    let valid = client
        .cookies(&cookies)
        .post_form(
            "/admin/showcase/schema",
            form_body(&[
                ("csrf_token", &csrf),
                ("name", "Ada Lovelace"),
                ("email", "ada@example.com"),
            ]),
        )
        .await;
    assert_eq!(valid.status(), 303, "valid submit redirects (PRG)");
    let flash = response_cookies(&valid);
    let followed = client
        .cookies(&cookies)
        .cookies(&flash)
        .get("/admin/showcase/schema")
        .await;
    let html = body_string(followed).await;
    assert!(
        html.contains("data-type=\"success\"") && html.contains("\">Validated</div>"),
        "valid submit should flash a success toast: {html}"
    );
}

#[tokio::test]
async fn showcase_resource_renders_derives_and_navigation() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/resource").await;
    assert!(
        response.status().is_success(),
        "resource showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
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
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/panel").await;
    assert!(
        response.status().is_success(),
        "panel showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
    assert!(
        html.contains("Panel::new"),
        "missing Panel snippet in {html}"
    );
    assert!(
        html.contains("/admin") && html.contains("/showcase"),
        "missing prefix variants in {html}"
    );
    // The prefix mapping is a real table, not the old raw `<table>` with
    // unstyled cells (GH #151 §7).
    assert!(
        html.contains(">input</th>") && html.contains(">prefix()</th>"),
        "prefix mapping should render as a table with headers: {html}"
    );
}

#[tokio::test]
async fn showcase_db_renders_memoized_loader() {
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/db").await;
    assert!(
        response.status().is_success(),
        "db showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
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
    let client = demo_client(&router).await;
    let response = client.get("/admin/showcase/table").await;
    assert!(
        response.status().is_success(),
        "table showcase status {}",
        response.status()
    );
    let html = body_string(response).await;
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
    // The demos are static previews (GH #151): labels and rows, with no sort
    // links or search chrome that would promise an interaction the page
    // ignores. The declarations live in the snippets.
    assert!(
        !html.contains("aria-sort") && !html.contains("Prefix search matches this column"),
        "showcase table demos must stay static, got {html}"
    );
    assert!(
        !html.contains("data-live-search"),
        "showcase table demos must not render the live host, got {html}"
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
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let response = client.get("/admin/users").await;
    let page1 = body_string(response).await;

    // Page 1 (name asc, 2 per page): Ada + Alan, not Grace; a real Next link.
    assert!(page1.contains("Ada Lovelace"), "page1 missing Ada: {page1}");
    assert!(page1.contains("Alan Turing"), "page1 missing Alan: {page1}");
    assert!(
        !page1.contains("Grace Hopper"),
        "page1 must not show Grace (page size 2): {page1}"
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
        html.contains("Prefix search matches this column"),
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
        html.contains("data-slot=\"field\""),
        "Resource::form should render TextInput field wrappers in {html}"
    );
    assert!(
        html.contains("border-border")
            && html.contains("bg-transparent")
            && html.contains("focus-visible:ring-ring"),
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
        html.matches("data-slot=\"field\"").count() >= 2,
        "Resource::form should have at least 2 fields (name, email) in {html}"
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
