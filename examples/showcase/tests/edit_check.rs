use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};

use crate::common::{
    assert_hydrate_keys_are_form_fields, body_string, demo_client, response_cookies, seeded_db,
    set_cookie_header,
};

#[tokio::test]
async fn edit_page_hydrates_and_updates() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

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
    let client = demo_client(&router, &db).await;
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
    let client = demo_client(&router, &db).await;
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
    let client = demo_client(&router, &db).await;
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
    let client = demo_client(&router, &db).await;

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
    let client = demo_client(&router, &db).await;

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
        // The submit carries no `media` discriminant (it predates GH #191), so
        // the codec falls back to the first variant — Image. Emptiness is not
        // consulted; a discriminant would decide.
        other => panic!("a submit with no discriminant reads as the first variant, got {other:?}"),
    }
}

/// GH #191: the edit form carries the **stored variant**, and a submit that
/// names a different one switches the value — even while the old variant's
/// payload is still filled in.
///
/// That is the case the hand-written reassembly got wrong: it picked the
/// variant from which payload columns happened to be non-empty, so a stale
/// `publication_canonical_url` silently outvoted the variant the user meant.
#[tokio::test]
async fn post_edit_switches_the_publication_variant_explicitly() {
    use showcase::models::{Media, Post, Publication};

    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the seeded post");
    assert!(
        matches!(post.publication, Publication::Published { .. }),
        "the fixture must start Published"
    );

    // Hydration: the stored variant reaches the form as its discriminant.
    let html = body_string(client.get(&format!("/admin/posts/{}/edit", post.id)).await).await;
    assert!(
        html.contains("name=\"publication\"") && html.contains("type=\"hidden\""),
        "the discriminant must ride the edit form, got {html}"
    );
    assert!(
        html.contains("value=\"2\""),
        "the stored Published variant must hydrate into the discriminant, got {html}"
    );

    // Submit Archived while leaving the Published payload filled in: the
    // discriminant decides, so the post is Archived and the stale canonical URL
    // is not what the row carries.
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/posts/{}/edit", post.id),
            format!(
                "title=Hello+Toasty&author_id={}&image_path={}&tags=rust&body=Body&\
                 status=published&featured=true&seo_title=Edited+SEO&seo_description=Desc&\
                 media=2&media_url=&media_alt=&media_video_url=hello-toasty.mp4&\
                 media_poster_url=p.jpg&media_poster_credit_author=Ada&\
                 media_poster_credit_licence=CC-BY&\
                 post_stats_word_count=10&post_stats_read_minutes=1&\
                 publication=3&publication_timestamp=2026-01-01T00:00:00Z&\
                 publication_canonical_url=https%3A%2F%2Fexample.com%2Fstale&\
                 publication_reason=superseded&csrf_token={csrf}",
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
    match saved.publication {
        Publication::Archived {
            archived_at,
            reason,
        } => {
            assert_eq!(archived_at, "2026-01-01T00:00:00Z");
            assert_eq!(reason, "superseded");
        }
        other => panic!(
            "the submitted discriminant must decide the variant, got {other:?} \
             (a filled canonical_url is not a vote)"
        ),
    }
    // The media discriminant round-trips unchanged: still the stored Video.
    assert!(
        matches!(saved.media, Media::Video { .. }),
        "an unchanged variant stays put"
    );
}

/// GH #191: the **create** form carries no discriminant (there is no stored
/// variant to hydrate), so a submission that fills a variant's payload creates
/// that variant — the pre-#191 behaviour, now driven by the keys the app schema
/// resolves rather than remembered column names.
///
/// Without this, every created post was `Scheduled` and the payload the author
/// typed was silently dropped: the hidden discriminant renders empty on create.
#[tokio::test]
async fn post_create_keeps_the_variant_its_payload_names() {
    use showcase::models::{Post, Publication};

    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let author = showcase::models::Author::all()
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("a seeded author");

    // The create page renders the discriminant empty — nothing to hydrate.
    let html = body_string(client.get("/admin/posts/create").await).await;
    assert!(
        html.contains("name=\"publication\""),
        "the create form must carry the discriminant, got {html}"
    );

    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Created+Published&author_id={}&image_path=created.jpg&tags=&body=Body&\
                 status=published&featured=false&seo_title=S&seo_description=D&\
                 media=1&media_url=/i.jpg&media_alt=alt&media_video_url=&\
                 media_poster_url=&media_poster_credit_author=&media_poster_credit_licence=&\
                 post_stats_word_count=1&post_stats_read_minutes=1&\
                 publication=&publication_timestamp=2026-03-01T00:00:00Z&\
                 publication_canonical_url=https%3A%2F%2Fexample.com%2Fnew&csrf_token={csrf}",
                author.id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "a valid create must redirect, got {}",
        resp.status()
    );

    let mut db_check = db.clone();
    let created = Post::filter(Post::fields().title().eq("Created Published".to_string()))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap()
        .expect("the created post");
    assert_eq!(
        created.publication,
        Publication::Published {
            published_at: "2026-03-01T00:00:00Z".to_string(),
            canonical_url: "https://example.com/new".to_string(),
        },
        "the submitted payload names the variant when no discriminant is posted"
    );
}

/// The typed leaves round-trip and refuse a bad number inline (GH #192).
///
/// `post_stats_word_count` / `post_stats_read_minutes` are `i64` columns bound
/// through `TextInput::typed`. Before this they were unbound and the record fn
/// parsed them with `unwrap_or(0)`, so `word_count=twelve` stored a zero.
#[tokio::test]
async fn post_edit_round_trips_typed_leaves_and_refuses_a_bad_number() {
    use showcase::models::Post;

    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the seeded post");

    // Hydration: the integer reaches the control through its own `Display`.
    let html = body_string(client.get(&format!("/admin/posts/{}/edit", post.id)).await).await;
    assert!(
        html.contains("name=\"post_stats_word_count\"")
            && html.contains(&format!("value=\"{}\"", post.post_stats.word_count)),
        "the typed integer must hydrate its control, got {html}"
    );

    let csrf = uuid::Uuid::new_v4().to_string();
    let body = |word_count: &str| {
        format!(
            "title=Hello+Toasty&author_id={}&image_path={}&tags=rust&body=Body&\
             status=published&featured=true&seo_title=Edited+SEO&seo_description=Desc&\
             media_url=/uploads/new.jpg&media_alt=Alt&media_video_url=&\
             media_poster_url=&media_poster_credit_author=&\
             post_stats_word_count={word_count}&post_stats_read_minutes=9&csrf_token={csrf}",
            post.author_id, post.image_path
        )
    };

    // A number the column cannot hold is a field error, not a silent zero.
    let resp = client
        .csrf(&csrf)
        .post_form(&format!("/admin/posts/{}/edit", post.id), body("twelve"))
        .await;
    assert!(
        resp.status().is_success(),
        "a bad number re-renders the form, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
    assert!(
        html.contains("`twelve` is not a valid whole number"),
        "the error names the offending input, got {html}"
    );
    let after_bad = Post::filter(Post::fields().id().eq(post.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        after_bad.post_stats.word_count, post.post_stats.word_count,
        "a rejected edit writes nothing"
    );

    // A number it can hold round-trips, and the stored value is the type's
    // spelling of it.
    let resp = client
        .csrf(&csrf)
        .post_form(&format!("/admin/posts/{}/edit", post.id), body("4242"))
        .await;
    assert!(
        resp.status().is_redirection(),
        "a valid number redirects, got {}",
        resp.status()
    );
    let after = Post::filter(Post::fields().id().eq(post.id))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        after.post_stats.word_count, 4242,
        "the typed value is stored"
    );
    assert_eq!(after.post_stats.read_minutes, 9, "its sibling too");
}
