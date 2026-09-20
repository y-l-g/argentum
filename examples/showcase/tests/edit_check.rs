use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

use crate::common::{
    TestClient, assert_hydrate_keys_are_form_fields, body_string, demo_client, response_cookies,
    seeded_db, set_cookie_header,
};

#[tokio::test]
async fn edit_page_hydrates_and_updates() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    // Get a user id
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap();
    let id = user.id.to_string();
    let edit_url = format!("/admin/users/{}/edit", id);

    // GET edit should be 200 with hydrated values
    let resp = client.get(&edit_url).await;
    assert!(
        resp.status().is_success(),
        "GET edit should be 200, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
    // GH #136 layer rule: core owns the field detail; the edit page pins
    // hydration — the stored values arrive in the form.
    assert!(
        html.contains(&user.name),
        "edit should contain hydrated name {}, got {}",
        user.name,
        html
    );
    assert!(
        html.contains(&user.email),
        "edit should contain hydrated email"
    );

    // Invalid POST should re-render with errors and not mutate
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(&edit_url, format!("name=&email=bad&csrf_token={csrf}"))
        .await;
    assert!(
        resp.status().is_success(),
        "invalid POST should re-render 200, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
    assert!(
        html.contains("is required") || html.contains("must be a valid email"),
        "should contain validation error, got {}",
        html
    );
    // Check DB not mutated
    let mut db_check = db.clone();
    let fresh = User::get_by_id(&mut db_check, &user.id).await.unwrap();
    assert_eq!(fresh.name, user.name, "should not mutate on invalid");

    // Valid POST should update and redirect with notification
    let resp = client
        .csrf(&csrf)
        .post_form(
            &edit_url,
            format!("name=Updated%20Name&email=updated%40example.com&csrf_token={csrf}",),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid POST should redirect, got {}",
        resp.status()
    );
    let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(
        loc.contains("/admin/users"),
        "redirect to list, got {}",
        loc
    );
    // Post/Redirect/Get with one-time semantics (GH #97, #126): 303, flash
    // cookie on the redirect, clean Location.
    assert_eq!(resp.status(), 303, "a completed update is a 303 PRG");
    assert!(
        !loc.contains("notification"),
        "the toast must not ride the query, got {loc}"
    );
    let flash = set_cookie_header(&resp, "__Host-argentum_notification")
        .expect("the flash cookie is set on the redirect");
    assert!(
        flash.contains("Updated"),
        "the flash carries the action, got {flash}"
    );
    // Follow redirect carrying the flash cookie and check the toast
    let resp2 = client.cookies(&response_cookies(&resp)).get(loc).await;
    let html2 = body_string(resp2).await;
    assert!(
        html2.contains("Updated"),
        "notification should survive, got {}",
        html2
    );
    // Check DB mutated
    let mut db_check2 = db.clone();
    let updated = User::get_by_id(&mut db_check2, &user.id).await.unwrap();
    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.email, "updated@example.com");
}

#[tokio::test]
async fn edit_404_for_unknown_or_wrong_tenant() {
    // GH #136 layer rule: core (`find_by_key_loads_one_row_scoped_and_404s_malformed`)
    // owns the loader unit; this pins the HTTP route. Wrong-tenant scoping
    // rides the same seam and is pinned in `tenancy_check.rs`
    // (`edit_with_wrong_tenant_yields_404_via_resource_query`).
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let fake_id = uuid::Uuid::new_v4().to_string();
    let resp = client.get(&format!("/admin/users/{}/edit", fake_id)).await;
    assert_eq!(
        resp.status(),
        404,
        "unknown id should be 404, got {}",
        resp.status()
    );
}

/// A forged edit POST must answer 403 before the advisory record lookup
/// (GH #144): the CSRF check runs first, so a nonexistent id cannot turn the
/// token-less 403 into a 404 existence oracle. (The 403-vs-404 distinction
/// makes this falsifiable: moving the verify back behind the load flips the
/// nonexistent-id answer to 404.)
#[tokio::test]
async fn edit_rejects_forged_post_before_probing_the_record() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let fake_id = uuid::Uuid::new_v4().to_string();
    let csrf = uuid::Uuid::new_v4().to_string();
    let cookie_mismatch = uuid::Uuid::new_v4().to_string();
    for (body, label) in [
        // Field value differs from the cookie value.
        (format!("name=x&csrf_token={csrf}"), "mismatched token"),
        ("name=x".to_string(), "missing token"),
    ] {
        let resp = client
            .csrf(&cookie_mismatch)
            .post_form(&format!("/admin/users/{fake_id}/edit"), body)
            .await;
        assert_eq!(
            resp.status(),
            403,
            "{label}: forged edit must 403 before the advisory lookup (never 404), got {}",
            resp.status()
        );
    }
}

#[tokio::test]
async fn edit_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        email: String,
    }

    struct DenyUpdateResource;
    impl Resource for DenyUpdateResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            true
        }
        fn can_update(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            false
        }
        fn table(cx: &topcoat::context::Cx) -> Table<DummyUser> {
            Table::r#for(cx)
                .id(|u: &DummyUser| u.id.to_string())
                .columns(TextColumn::r#for(
                    DummyUser::fields().name(),
                    |u: &DummyUser| u.name.clone(),
                ))
        }
        fn form(_cx: &topcoat::context::Cx) -> Schema {
            Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
        }
        fn hydrate_form_values(_r: &DummyUser) -> std::collections::HashMap<String, String> {
            std::collections::HashMap::new()
        }
    }

    let mut db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let rec = toasty::create!(DummyUser {
        name: "x".to_string(),
        email: "x@example.com".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .auth(argentum_core::Auth::disabled())
        .resource::<DenyUpdateResource>()
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);
    let slug = DenyUpdateResource::slug();
    let edit_url = format!("/admin/{}/{}/edit", slug, rec.id);

    // GET should be 403
    let resp = client.get(&edit_url).await;
    assert_eq!(
        resp.status(),
        403,
        "GET edit should be 403 when update denied, got {}",
        resp.status()
    );

    // POST should also be 403
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(&edit_url, format!("name=y&csrf_token={csrf}"))
        .await;
    assert_eq!(
        resp.status(),
        403,
        "POST edit should be 403 when update denied, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn update_record_keeps_absent_fields() {
    use argentum_core::Resource;
    use showcase::app::UserResource;
    use std::collections::HashMap;

    let db = seeded_db().await;
    let cx = topcoat::context::CxTestBuilder::new()
        .app_context(db.clone())
        .build();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap().clone();
    // Only email submitted: name must keep its stored value (GH #89).
    let mut values = HashMap::new();
    values.insert("email".to_string(), "kept@example.com".to_string());
    let mut ex = argentum_core::db::db(&cx);
    UserResource::update_record(&cx, user.clone(), values, &mut ex)
        .await
        .unwrap();
    let mut db_check = db.clone();
    let fresh = User::get_by_id(&mut db_check, &user.id).await.unwrap();
    assert_eq!(fresh.email, "kept@example.com");
    assert_eq!(
        fresh.name, user.name,
        "absent fields must not be blanked, got {}",
        fresh.name
    );
}

#[tokio::test]
async fn hydrate_form_values_match_schema_fields() {
    use showcase::app::UserResource;

    let db = seeded_db().await;
    let cx = topcoat::context::CxTestBuilder::new()
        .app_context(db.clone())
        .build();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap();
    // Every hydrated key must be a declared form field (GH #89): a renamed
    // lens without an updated string literal would render blank and break
    // the unique unchanged-skip.
    assert_hydrate_keys_are_form_fields::<UserResource>(&cx, user);
}

#[tokio::test]
async fn edit_sso_managed_user_is_forbidden() {
    // Row-level Policy on the update path: Ken's page and POST both deny.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    let ken = showcase::models::User::filter(
        showcase::models::User::fields()
            .name()
            .eq("Ken Thompson".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("Ken seed");
    let resp = client.get(&format!("/admin/users/{}/edit", ken.id)).await;
    assert_eq!(
        resp.status(),
        403,
        "protected row edit page must be forbidden, got {}",
        resp.status()
    );
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/users/{}/edit", ken.id),
            format!("name=Ken+Hacked&email=ken%40example.com&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "protected row edit POST must be forbidden, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let unchanged = showcase::models::User::filter(
        showcase::models::User::fields()
            .email()
            .eq("ken@example.com".to_string()),
    )
    .first()
    .exec(&mut db_check)
    .await
    .unwrap()
    .expect("Ken unchanged");
    assert_eq!(unchanged.name, "Ken Thompson");
}

#[tokio::test]
async fn post_body_renders_as_a_textarea() {
    // GH #184 §9: a post body is prose, so the edit form renders a
    // `<textarea>` instead of the one-line input it used to share with `title`.
    use showcase::models::Post;

    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    let mut db_q = db.clone();
    let post = Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a seeded post");

    let resp = client.get(&format!("/admin/posts/{}/edit", post.id)).await;
    assert_eq!(resp.status(), 200, "GET post edit should be 200");
    let html = body_string(resp).await;

    // Slice this field's control: from its own label to the closing
    // `</textarea>`. Slicing on the `field` wrapper would swallow the
    // neighbouring `title` input, since the wrapper carries no id of its own.
    let label = html
        .find("for=\"body\"")
        .expect("the body field must render a label");
    let close = html
        .find("</textarea>")
        .expect("the body field must render a textarea");
    let field = &html[label..close];
    assert!(
        field.contains("<textarea"),
        "the body field must be a textarea, got {field}"
    );
    assert!(
        !field.contains("<input"),
        "the body field must not be an input, got {field}"
    );
    assert!(
        field.contains(&post.body),
        "the textarea must carry the stored body, got {field}"
    );
}

/// GH #185: an embedded field's control posts its **flattened column**, and
/// saving it actually persists — the whole point of resolving the lens through
/// the app schema rather than the model alone.
#[tokio::test]
async fn post_edit_binds_and_saves_embedded_fields() {
    use showcase::models::{Media, Post};

    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the seeded post");

    // The form renders the flattened names, and the stored values hydrate into
    // those controls. `seo_title` is the embedded struct's leaf.
    let resp = client.get(&format!("/admin/posts/{}/edit", post.id)).await;
    assert_eq!(resp.status(), 200);
    let html = body_string(resp).await;
    assert!(
        html.contains("name=\"seo_title\""),
        "the embedded leaf must render as its flattened column, got {html}"
    );
    assert!(
        html.contains(&format!("value=\"{}\"", post.seo.title)),
        "the stored embedded value must hydrate, got {html}"
    );
    // Three levels deep, inside an enum variant.
    assert!(
        html.contains("name=\"media_poster_credit_author\""),
        "a struct nested in a variant must flatten to its column, got {html}"
    );

    // Save with new embedded values; the flattened columns must reach the
    // record fn and land in the row.
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/posts/{}/edit", post.id),
            format!(
                "title=Hello+Toasty&author_id={}&image_path={}&tags=rust&body=Body&\
                 status=published&featured=true&seo_title=Edited+SEO&seo_description=Desc&\
                 media_url=/uploads/new.jpg&media_alt=Alt&media_video_url=&\
                 media_poster_url=&media_poster_credit_author=&csrf_token={csrf}",
                post.author_id, post.image_path
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "a valid edit must redirect, got {}",
        resp.status()
    );

    let mut db_check = db.clone();
    let saved = Post::filter(Post::fields().id().eq(post.id))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap()
        .expect("the post");
    assert_eq!(
        saved.seo.title, "Edited SEO",
        "the embedded struct's leaf must persist"
    );
    assert_eq!(saved.seo.description, "Desc");
    match saved.media {
        Media::Image { url, alt } => {
            assert_eq!(url, "/uploads/new.jpg", "the variant payload must persist");
            assert_eq!(alt, "Alt");
        }
        other => panic!("emptying the video payload must select Image, got {other:?}"),
    }
}
