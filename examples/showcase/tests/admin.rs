use topcoat::view::ViewExt;

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
    // Toast stack: the shadcn/Sonner surface, fixed bottom-right, with the
    // live shard mounted inside (GH #154 §3).
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
    // Dialog: alert_dialog with Primary/Destructive buttons, opened through a
    // bound signal and kept in step by the element's `@close` handler.
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
    assert!(
        html.contains("data-topcoat-bind:open") && html.contains("data-topcoat-on:close"),
        "dialog must open from a signal and sync on close: {html}"
    );
    // Ensure no ac-* remains in this showcase. Match the class attribute
    // (not bare "ac-"), because the CSRF token is a random UUID that can
    // contain the same substring and flake the assertion.
    assert!(
        !html.contains("ac-showcase") && !html.contains("class=\"ac-"),
        "ac-* should not remain in dialog showcase, got {html}"
    );
}

/// The dialog page's toast demo is in-place now (GH #154 §3): the buttons
/// call a procedure that returns the real notification payload, and the
/// shell's `live_toaster` shard renders it — no form POST, no redirect, no
/// flash cookie.
#[tokio::test]
async fn showcase_dialog_toast_demo_mounts_a_notification_in_place() {
    use topcoat::runtime::{Shard, ShardId};

    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let identity = "A".repeat(22);
    let page = client.get("/admin/showcase/dialog").await;
    let html = body_string(page).await;
    // The old transport is gone: no notify form, and each variant is a
    // button whose handler writes the procedure result into a signal.
    assert!(
        !html.contains("action=\"/admin/showcase/dialog/notify\""),
        "the PRG toast form must be gone: {html}"
    );
    assert!(
        html.matches("data-topcoat-on:click").count() >= 4,
        "every toast button must carry a click handler: {html}"
    );
    // The procedure reference is embedded in those handlers; `ProcedureId`
    // has no accessor, so read it from the page.
    let procedure_id: String = {
        let needle = "&quot;t&quot;:&quot;Procedure&quot;,&quot;id&quot;:&quot;";
        let at = html.find(needle).expect("procedure reference") + needle.len();
        let end = at + html[at..].find('&').expect("procedure id end");
        html[at..end].to_string()
    };
    for (status, title, description) in [
        (
            "success",
            "User created",
            "Ada Lovelace was added successfully.",
        ),
        ("info", "Heads up", "The record already exists."),
        ("warning", "Careful", "This action changes stored data."),
        ("error", "Something failed", "Nothing was changed."),
    ] {
        let response = client
            .post_json(
                &format!("/_topcoat/runtime/procedures/{procedure_id}"),
                format!(r#"["{status}"]"#),
                &identity,
            )
            .await;
        assert_eq!(response.status(), 200, "{status} procedure");
        let body = body_string(response).await;
        assert_eq!(
            body,
            format!(r#"["{status}","{title}","{description}"]"#),
            "the procedure must return the notification fields"
        );
    }

    // The shell's live toaster renders the payload the page writes into its
    // signals, with a per-mount id so a repeat remounts.
    let shard: ShardId = Shard::id(&argentum_core::notification::live_toaster);
    let signal =
        |id: u128, value: &str| format!(r#"{{"t":"Signal","id":"{id:032x}","v":"{value}"}}"#);
    let serial = |value: &str| {
        format!(
            r#"{{"t":"Signal","id":"{:032x}","v":{{"t":"u64","bits":64,"v":"{value}"}}}}"#,
            9u128
        )
    };
    let args = format!(
        r#"{{"args":[{}, {}, {}, {}],"signals":{{}}}}"#,
        signal(1, "success"),
        signal(2, "User created"),
        signal(3, "Ada Lovelace was added successfully."),
        serial("4"),
    );
    let response = client
        .post_json(
            &format!("/_topcoat/runtime/shards/{}", shard.as_str()),
            args,
            &identity,
        )
        .await;
    assert_eq!(response.status(), 200, "live toaster shard");
    let html = body_string(response).await;
    assert!(
        html.contains("data-type=\"success\"")
            && html.contains(">User created</div>")
            && html.contains(">Ada Lovelace was added successfully.</div>")
            && html.contains("id=\"live-toast-4\""),
        "the shard must mount the real Sonner toast: {html}"
    );
    assert!(
        !html.contains("data-removed=\"true\""),
        "a fresh mount starts unmounted for the enter transition: {html}"
    );
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
    // The live-validation form is a shard with signal-bound inputs and an
    // in-place submit; no POST form or `novalidate` remains (GH #154 §4).
    assert!(
        html.contains("data-topcoat-bind:value")
            && html.contains("data-topcoat-on:submit")
            && html.contains("data-topcoat-on:input")
            && html.contains("aria-live=\"polite\"")
            && html.contains("Live validation"),
        "missing live validation form in {html}"
    );
    assert!(
        !html.contains("action=\"/admin/showcase/schema\"") && !html.contains("novalidate"),
        "the PRG form must be gone: {html}"
    );
}

/// The validation demo is in-place now (GH #154 §4): the procedure returns
/// the per-field errors, the submit handler writes them into signals, and the
/// shard re-renders the fields with the existing error slots — no POST, no
/// redirect, no full-page render.
#[tokio::test]
async fn showcase_schema_validation_form_reports_errors_without_a_reload() {
    use showcase::pages::showcase::schema::live_validation_form;
    use topcoat::runtime::{Shard, ShardId};

    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let identity = "A".repeat(22);
    let page = client.get("/admin/showcase/schema").await;
    let html = body_string(page).await;
    // `ProcedureId` has no accessor; read the embedded reference.
    let procedure_id: String = {
        let needle = "&quot;t&quot;:&quot;Procedure&quot;,&quot;id&quot;:&quot;";
        let at = html.find(needle).expect("procedure reference") + needle.len();
        let end = at + html[at..].find('&').expect("procedure id end");
        html[at..end].to_string()
    };
    // The procedure applies `Schema::validate` and returns one error string
    // per field (empty = valid).
    for (name, email, expected) in [
        (
            "",
            "not-an-email",
            r#"["Name is required","Email must be a valid email"]"#,
        ),
        ("Ada Lovelace", "ada@example.com", r#"["",""]"#),
    ] {
        let response = client
            .post_json(
                &format!("/_topcoat/runtime/procedures/{procedure_id}"),
                format!(r#"["{name}","{email}"]"#),
                &identity,
            )
            .await;
        assert_eq!(response.status(), 200, "validation procedure");
        assert_eq!(body_string(response).await, expected);
    }

    // The shard renders the fields from the page's signals: with the error
    // signals set, the errors appear under their fields with the invalid
    // chrome; with them empty, the form is clean.
    let shard: ShardId = Shard::id(&live_validation_form);
    let signal =
        |id: u128, value: &str| format!(r#"{{"t":"Signal","id":"{id:032x}","v":"{value}"}}"#);
    let serial = |value: &str| {
        format!(
            r#"{{"t":"Signal","id":"{:032x}","v":{{"t":"u64","bits":64,"v":"{value}"}}}}"#,
            9u128
        )
    };
    let args = |name: &str, email: &str, name_error: &str, email_error: &str| {
        format!(
            r#"{{"args":[{}, {}, {}, {}, {}, {}, {}, {}],"signals":{{}}}}"#,
            signal(1, name),
            signal(2, email),
            signal(3, name_error),
            signal(4, email_error),
            signal(5, ""),
            signal(6, ""),
            signal(7, ""),
            serial("0"),
        )
    };
    let response = client
        .post_json(
            &format!("/_topcoat/runtime/shards/{}", shard.as_str()),
            args(
                "Ada Lovelace",
                "not-an-email",
                "Name is required",
                "Email must be a valid email",
            ),
            &identity,
        )
        .await;
    assert_eq!(response.status(), 200, "live validation shard");
    let html = body_string(response).await;
    for needle in [
        "Name is required",
        "Email must be a valid email",
        "ac-field--error",
        "aria-invalid=\"true\"",
        "value=\"not-an-email\"",
        "data-topcoat-bind:value",
        "data-topcoat-on:submit",
        "aria-live=\"polite\"",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    // A valid render has no error chrome and keeps the typed value.
    let response = client
        .post_json(
            &format!("/_topcoat/runtime/shards/{}", shard.as_str()),
            args("Ada Lovelace", "ada@example.com", "", ""),
            &identity,
        )
        .await;
    let html = body_string(response).await;
    assert!(
        !html.contains("ac-field--error")
            && !html.contains("Name is required")
            && html.contains("value=\"ada@example.com\"")
            && html.contains("aria-invalid=\"false\""),
        "a valid submit must render clean fields: {html}"
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
    // The demos are live (GH #154 §2): the searchable and composition
    // sections each render the signal-backed input, and the grid of every
    // interactive demo arrives through the `table_demo` shard.
    assert_eq!(
        html.matches("data-live-search").count(),
        2,
        "searchable + composition demos must render the live input: {html}"
    );
    assert!(
        html.matches("::topcoat::shard::start").count() >= 3,
        "the searchable, sortable, and composition demos render a shard: {html}"
    );
    // Sortable columns render real sort controls: the inactive pointer and
    // aria-sort="none" on the two tables that declare them.
    assert_eq!(
        html.matches("aria-sort=\"none\"").count(),
        2,
        "sortable demos must render clickable headers: {html}"
    );
    assert!(
        html.contains("sort=name&amp;dir=asc"),
        "missing sort link in {html}"
    );
    assert!(
        html.contains("Ada Lovelace") || html.contains("Name"),
        "missing table rows in {html}"
    );
}

/// The page-level live seam (GH #154 §2): the `table_demo` shard reloads and
/// re-renders one demo grid from the page-owned signals — search filters,
/// sort orders, and the swapped controls stay bound to the same signals.
#[tokio::test]
async fn showcase_table_demo_shard_filters_and_sorts_in_place() {
    use showcase::pages::showcase::table::table_demo;
    let db = seeded_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let identity = "A".repeat(22);
    let shard = topcoat::runtime::Shard::id(&table_demo);
    let uri = format!("/_topcoat/runtime/shards/{}", shard.as_str());
    let sig = |n: u8, v: &str| format!(r#"{{"t":"Signal","id":"{:032x}","v":"{v}"}}"#, n);
    let args = |kind: &str, q: &str, sort: &str, dir: &str| {
        format!(
            r#"{{"args":["{kind}",{}, {}, {}],"signals":{{}}}}"#,
            sig(1, q),
            sig(2, sort),
            sig(3, dir)
        )
    };

    // Search filters the grid; the page's toolbar stays out of the swapped
    // region (the page owns it).
    let response = client
        .post_json(&uri, args("searchable", "Ada", "", "asc"), &identity)
        .await;
    assert_eq!(response.status(), 200, "shard render");
    let html = body_string(response).await;
    assert!(
        html.contains("Ada Lovelace") && !html.contains("Grace Hopper"),
        "search must filter rows: {html}"
    );
    assert!(
        !html.contains("name=\"q\""),
        "the swapped grid must not duplicate the page's search input: {html}"
    );

    // A term with no matches renders the honest empty state.
    let response = client
        .post_json(&uri, args("searchable", "zzz", "", "asc"), &identity)
        .await;
    let html = body_string(response).await;
    assert!(
        html.contains("No prefix matches"),
        "empty search must say so: {html}"
    );

    // Sort orders rows and keeps the header control bound to the signals.
    let response = client
        .post_json(&uri, args("sortable", "", "name", "desc"), &identity)
        .await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    let grace = html.find("Grace Hopper").expect("Grace row");
    let ada = html.find("Ada Lovelace").expect("Ada row");
    assert!(grace < ada, "descending sort must lead with Grace: {html}");
    assert!(
        html.contains("aria-sort=\"descending\"")
            && html.contains("data-topcoat-on:click")
            && html.contains("sort=name"),
        "live header must carry its bound sort link: {html}"
    );

    // An unknown kind is client input: reject it, never fall back.
    let response = client
        .post_json(&uri, args("nope", "", "", "asc"), &identity)
        .await;
    assert_eq!(response.status(), 400, "unknown demo kind");
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
        !html.contains("Prefix search matches this column"),
        "searchable headers must not carry a loupe, got {html}"
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
