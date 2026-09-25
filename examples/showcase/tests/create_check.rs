use http::header::{LOCATION, SET_COOKIE};
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

use crate::common::{
    TestClient, body_string, demo_client, response_cookies, seeded_db, set_cookie_header,
    user_count,
};

#[tokio::test]
async fn create_page_serves_the_declared_fields() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get("/admin/users/create").await;
    assert!(resp.status().is_success(), "GET create should be 200");
    let html = body_string(resp).await;
    // GH #136 layer rule: core (`text_input_renders_with_label_and_ac_field`)
    // owns the field detail (wrapper, Tokens, for/id, error slot); this pins
    // the HTTP wiring — the create page serves the declared fields.
    assert!(
        html.contains("<form"),
        "missing form in {}",
        &html[..html.len().min(2000)]
    );
    assert!(
        html.contains("name=\"name\"") && html.contains("name=\"email\""),
        "missing declared fields in {}",
        &html[..html.len().min(2000)]
    );
}

/// A rejected submission re-renders with the field errors, and writes nothing.
#[tokio::test]
async fn create_invalid_submission_rerenders_with_inline_errors() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let before = user_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=&email=not-an-email&csrf_token={csrf}"),
        )
        .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "invalid POST should re-render 200, not redirect, got {status}"
    );
    assert!(
        html.contains("is required"),
        "should contain is required error, got {html}"
    );
    assert!(
        html.contains("must be a valid email"),
        "should contain email error, got {html}"
    );
    assert_eq!(
        user_count(&db).await,
        before,
        "an invalid create must not add a user"
    );
}

/// A valid submission is a Post/Redirect/Get with one-time flash semantics
/// (#126): 303, clean Location, the toast on the flash cookie, and the
/// follow-up response consuming it.
#[tokio::test]
async fn create_valid_redirects_with_a_one_time_flash() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=New%20User&email=new%40example.com&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), 303, "a completed create is a 303");
    let loc = resp
        .headers()
        .get(LOCATION)
        .expect("missing Location")
        .to_str()
        .expect("a text Location")
        .to_string();
    assert!(
        loc.starts_with("/admin/users"),
        "redirect to list, got {loc}"
    );
    assert!(
        !loc.contains("notification"),
        "the toast must not ride the query, got {loc}"
    );
    let cookies: Vec<String> = resp
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    assert!(
        cookies
            .iter()
            .any(|c| c.contains("__Host-argentum_notification")),
        "the flash cookie must be set on the redirect, got {cookies:?}"
    );

    // Follow the redirect, carrying whatever cookies the POST set (the flash
    // cookie included — the Location query carries no notification param).
    let resp2 = client.cookies(&response_cookies(&resp)).get(&loc).await;
    assert!(
        resp2.status().is_success(),
        "GET list after create should be 200"
    );
    // The shell consumed the one-time flash: the follow-up response clears it.
    let cleared = set_cookie_header(&resp2, "__Host-argentum_notification")
        .expect("following the redirect must consume the flash");
    assert!(
        cleared.contains("Max-Age=0") || cleared.contains("Expires=Thu, 01 Jan 1970"),
        "the flash is one-time, got {cleared}"
    );
}

#[tokio::test]
async fn create_valid_persists_the_new_user_and_toasts_it() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let before = user_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=New%20User&email=new%40example.com&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), 303, "a completed create is a 303");
    let loc = resp
        .headers()
        .get(LOCATION)
        .expect("missing Location")
        .to_str()
        .expect("a text Location")
        .to_string();

    let mut db_check = db.clone();
    assert_eq!(
        user_count(&db).await,
        before + 1,
        "a valid create adds exactly one user"
    );
    let new_user = User::filter(User::fields().email().eq("new@example.com".to_string()))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap();
    assert!(new_user.is_some(), "new user should exist");

    let resp2 = client.cookies(&response_cookies(&resp)).get(&loc).await;
    let html2 = body_string(resp2).await;
    assert!(
        html2.contains("data-sonner-toaster"),
        "missing the toast stack in {}",
        html2
    );
    assert!(
        html2.contains("Created"),
        "the shell must render the consumed toast in {}",
        html2
    );
    assert!(
        html2.contains("data-sonner-toast") && html2.contains("data-type=\"success\""),
        "missing the success toast surface, got {}",
        html2
    );
}

#[tokio::test]
async fn create_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model, Clone)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        email: String,
    }

    struct DenyCreateResource;
    impl Resource for DenyCreateResource {
        type Model = DummyUser;
        fn can_create(_cx: &topcoat::context::Cx) -> bool {
            false
        }
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn table(cx: &topcoat::context::Cx) -> Table<DummyUser> {
            Table::r#for(cx)
                .id(|u: &DummyUser| u.id.to_string())
                .pk(|u: &DummyUser| u.id.to_string())
                .columns(TextColumn::r#for(
                    DummyUser::fields().name(),
                    |u: &DummyUser| u.name.clone(),
                ))
        }
        fn form(_cx: &topcoat::context::Cx) -> Schema {
            Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
        }
        async fn create_record(
            _cx: &topcoat::context::Cx,
            _values: std::collections::HashMap<String, String>,
            _ex: &mut dyn toasty::Executor,
        ) -> topcoat::Result<DummyUser> {
            // `can_create` denies before the handler ever calls this, so there
            // is no row to return (a create returns what it wrote).
            Err(std::io::Error::other("unreachable: create is denied by policy").into())
        }
    }

    let db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .auth(argentum_core::Auth::disabled())
        .resource::<DenyCreateResource>()
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);

    let slug = DenyCreateResource::slug();
    let create_url = format!("/admin/{}/create", slug);
    // GET create should be 403
    let resp = client.get(&create_url).await;
    assert_eq!(resp.status(), 403, "GET create should be 403 when denied");

    // POST should also be 403 and not create
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(&create_url, format!("name=test&csrf_token={csrf}"))
        .await;
    assert_eq!(
        resp.status(),
        403,
        "POST create should be 403 when denied, got {}",
        resp.status()
    );
    // Check DB still empty
    let mut db_check = db.clone();
    let count = DummyUser::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 0, "should not create when denied");
}

#[tokio::test]
async fn create_post_with_unknown_keys_is_bad_request() {
    // GH #89 allow-list: role/tenant_id smuggling is a 400 at the framework
    // layer, never silently ignored.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = user_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client.csrf(&csrf).post_form("/admin/users/create", format!(
            "name=Sneaky&email=sneaky%40example.com&role=admin&tenant_id=victim&csrf_token={csrf}"
        ))
    .await;
    assert_eq!(
        resp.status(),
        400,
        "unknown POST keys must be 400, got {}",
        resp.status()
    );
    assert_eq!(
        user_count(&db).await,
        before,
        "smuggled POST must not create"
    );
}

#[tokio::test]
async fn users_create_duplicate_email_shows_taken() {
    // The declared unique() field re-renders inline instead of writing.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = user_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=Copycat&email=ada%40example.com&csrf_token={csrf}"),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "duplicate POST must re-render 200, got {}",
        resp.status()
    );
    // The inline wording is `panel::forms`'s; this pins that the duplicate
    // re-renders instead of writing.
    assert_eq!(
        user_count(&db).await,
        before,
        "duplicate POST must not create"
    );
}

#[tokio::test]
async fn users_create_static_selects_set_role_and_active() {
    // Static-options Selects: the role vocabulary and the Yes/No active pair.
    // Relationship Selects live on the post and comment forms.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get("/admin/users/create").await;
    let html = body_string(resp).await;
    assert!(html.contains("Profile"), "missing profile section: {html}");
    assert!(
        html.contains("name=\"role\""),
        "missing role select: {html}"
    );
    assert!(
        html.contains("name=\"active\""),
        "missing active select: {html}"
    );

    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!(
                "name=New+Admin&email=newadmin%40example.com&role=admin&active=false&csrf_token={csrf}"
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid static-select POST must redirect, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let created = User::filter(
        User::fields()
            .email()
            .eq("newadmin@example.com".to_string()),
    )
    .first()
    .exec(&mut db_check)
    .await
    .unwrap()
    .expect("created user");
    assert_eq!(created.role, "admin");
    assert!(!created.active);
}

/// GH #295: a write that fails after validation answers a 500 whose body is
/// Topcoat's plain text, so the failure toast cannot render there. The flash
/// cookie rides the 500 response and the toast appears on the next panel page.
/// `notify_write_failure`'s doc comment describes this delivery.
#[tokio::test]
async fn a_failed_write_toasts_on_the_next_panel_page() {
    use std::collections::HashMap;

    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};
    use topcoat::context::Cx;

    #[derive(Debug, toasty::Model, Clone)]
    struct Widget {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }
    struct FailingResource;
    impl Resource for FailingResource {
        type Model = Widget;
        fn slug() -> String {
            "widgets".to_string()
        }
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_create(_cx: &Cx) -> bool {
            true
        }
        fn table(cx: &Cx) -> Table<Widget> {
            Table::r#for(cx)
                .id(|w: &Widget| w.id.to_string())
                .pk(|w: &Widget| w.id.to_string())
                .paginate(25)
                .columns(TextColumn::r#for(Widget::fields().name(), |w: &Widget| {
                    w.name.clone()
                }))
        }
        fn form(_cx: &Cx) -> Schema {
            Schema::new(TextInput::r#for(Widget::fields().name()))
        }
        async fn create_record(
            _cx: &Cx,
            _values: HashMap<String, String>,
            _ex: &mut dyn toasty::Executor,
        ) -> topcoat::Result<Widget> {
            // Validation passed; the write itself did not land.
            Err(std::io::Error::other("the write did not land").into())
        }
    }

    let db = Db::builder()
        .models(toasty::models!(Widget))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db)
        .auth(argentum_core::Auth::disabled())
        .resource::<FailingResource>()
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);

    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/widgets/create",
            format!("name=Widget&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        500,
        "a failed write is a 500, got {}",
        resp.status()
    );
    // The flash cookie rides the 500 response...
    let flash = set_cookie_header(&resp, "__Host-argentum_notification")
        .expect("the flash cookie must ride the 500 response");
    let cookie_value = flash
        .split(';')
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_string())
        .expect("the Set-Cookie names a value");
    // ...whose body is Topcoat's plain text, so it renders no toast.
    let body = body_string(resp).await;
    assert!(
        !body.contains("data-sonner-toast"),
        "the 500 body is plain text, so it renders no toast: {body}"
    );

    // The next panel page consumes the flash and renders the toast.
    let page = client
        .cookie("__Host-argentum_notification", &cookie_value)
        .get("/admin/widgets")
        .await;
    assert!(
        page.status().is_success(),
        "the list page after the failure must answer, got {}",
        page.status()
    );
    let html = body_string(page).await;
    assert!(
        html.contains("data-type=\"error\""),
        "the next panel page must render the failure toast: {html}"
    );
}
