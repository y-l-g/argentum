//! The public blog (GH #247): `/blog` and `/blog/{id}` answer with no session,
//! a draft is invisible on both, and the pages query the model directly.
//!
//! Every anonymous request here goes through [`TestClient::new`], which attaches
//! no cookie and no tenant — the point of the suite. The one signed-in client
//! proves a session changes nothing, and the panel request in the first test is
//! the control: the same router still gates `/admin`, so a 200 on `/blog` is the
//! blog being public rather than the gate being off.

use showcase::{
    app::router_for_tests as router,
    models::{Author, DEMO_TENANT, Media, Post, PostStats, Publication, Seo},
};

use crate::common::{TestClient, body_string, demo_client, empty_schema_db, full_db};

/// The one published seed post.
async fn published_post(db: &toasty::Db) -> Post {
    let mut db = db.clone();
    Post::filter(Post::fields().status().eq("published".to_string()))
        .first()
        .exec(&mut db)
        .await
        .expect("query the published seed post")
        .expect("the seed publishes one post")
}

/// The seeded draft, which the list must hide and the detail page must 404.
async fn draft_post(db: &toasty::Db) -> Post {
    let mut db = db.clone();
    Post::filter(Post::fields().title().eq("Second Post".to_string()))
        .first()
        .exec(&mut db)
        .await
        .expect("query the draft seed post")
        .expect("the seed creates the Second Post draft")
}

/// The path a post is served at, from the id the record carries.
fn post_path(post: &Post) -> String {
    format!("/blog/{}", post.id)
}

/// Create a published post carrying `title` and `excerpt`, written by `author`,
/// dated `created_at`.
async fn create_published(
    db: &toasty::Db,
    index: u128,
    title: &str,
    excerpt: &str,
    created_at: &str,
    author: &Author,
) {
    let mut db = db.clone();
    toasty::create!(Post {
        id: uuid::Uuid::from_u128(0x9000 + index),
        tenant_id: DEMO_TENANT,
        title: title.to_string(),
        body: format!("Body of {title}."),
        status: "published".to_string(),
        featured: false,
        created_at: created_at.parse::<jiff::Timestamp>().expect("a timestamp"),
        image_path: String::new(),
        tags: String::new(),
        seo: Seo {
            title: String::new(),
            description: excerpt.to_string(),
        },
        publication: Publication::Published {
            published_at: String::new(),
            canonical_url: String::new(),
        },
        media: Media::Image {
            url: String::new(),
            alt: String::new(),
        },
        post_stats: PostStats {
            word_count: 0,
            read_minutes: 0,
        },
        author_id: author.id,
    })
    .exec(&mut db)
    .await
    .expect("create a published post");
}

/// The seed's one published post is dated 2024-01-15, so these two bracket it:
/// `older` is after it and `newer` is after that.
const OLDER: &str = "2024-03-01T09:00:00Z";
const NEWER: &str = "2024-05-01T09:00:00Z";

#[tokio::test]
async fn blog_list_and_detail_are_public() {
    let db = full_db().await;
    let router = router(db.clone());
    // No cookie, no session, no tenant: a first-time visitor.
    let client = TestClient::new(&router);
    let post = published_post(&db).await;

    let list = client.get("/blog").await;
    assert_eq!(list.status(), 200, "the list must be public");
    let list_html = body_string(list).await;

    let detail = client.get(&post_path(&post)).await;
    assert_eq!(detail.status(), 200, "the detail page must be public");
    let detail_html = body_string(detail).await;

    // The panel on the same router still redirects an anonymous visitor, so
    // the 200s above are the blog's own public routes.
    let admin = client.get("/admin/posts").await;
    assert!(
        admin.status().is_redirection(),
        "the panel stays gated, got {}",
        admin.status()
    );

    // Both pages are the blog's own document, not a nested admin shell, and
    // they carry no panel chrome — the resource loaders are panel-scoped.
    for (name, html) in [("list", &list_html), ("detail", &detail_html)] {
        assert!(
            html.contains("<!DOCTYPE html>"),
            "the {name} page must render a complete document: {html}"
        );
        assert!(
            html.contains("Published with Argentum."),
            "the {name} page must render the public layout: {html}"
        );
        assert!(
            !html.contains("data-sidebar=\"sidebar\""),
            "the {name} page must not render the admin shell: {html}"
        );
    }
}

#[tokio::test]
async fn blog_list_shows_published_posts_with_author_date_and_excerpt() {
    let db = full_db().await;
    let router = router(db.clone());
    let post = published_post(&db).await;

    let response = TestClient::new(&router).get("/blog").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;

    assert!(
        html.contains(&post.title),
        "the list must name the published post: {html}"
    );
    assert!(
        html.contains(&format!("href=\"{}\"", post_path(&post))),
        "the list must link the post's page: {html}"
    );
    // The author comes from the query's include, not a per-row load.
    assert!(
        html.contains("Ada Author"),
        "the list must name the author: {html}"
    );
    assert!(
        html.contains(&post.created_at.strftime("%Y-%m-%d").to_string()),
        "the list must date the post: {html}"
    );
    assert!(
        html.contains(&post.seo.description),
        "the list must show the excerpt: {html}"
    );
}

#[tokio::test]
async fn blog_detail_renders_body_cover_and_seo_description() {
    let db = full_db().await;
    let router = router(db.clone());
    let post = published_post(&db).await;

    let response = TestClient::new(&router).get(&post_path(&post)).await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;

    assert!(
        html.contains(&post.title),
        "the detail page must carry the title: {html}"
    );
    assert!(
        html.contains(&post.body),
        "the detail page must render the body: {html}"
    );
    assert!(
        html.contains(&post.seo.description),
        "the detail page must render the SEO description: {html}"
    );
    assert!(
        html.contains("Ada Author"),
        "the detail page must name the author: {html}"
    );
    // The seeded cover is a bare filename no route serves, so the page renders
    // no image rather than a broken one.
    assert!(
        !html.contains("<img"),
        "a filename that names no route must not render as a cover: {html}"
    );
}

#[tokio::test]
async fn a_servable_cover_renders_as_an_image() {
    let db = full_db().await;
    let router = router(db.clone());
    let post = published_post(&db).await;

    // The demo uploader stores the URL it returned, a rooted path the panel
    // serves (GH #188) — the shape a cover has once it is a real upload.
    let stored = "/uploads/cover.png".to_string();
    let mut db_q = db.clone();
    Post::filter(Post::fields().id().eq(post.id))
        .update()
        .image_path(stored.clone())
        .exec(&mut db_q)
        .await
        .expect("point the post at a servable cover");

    let response = TestClient::new(&router).get(&post_path(&post)).await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    assert!(
        html.contains(&format!("src=\"{stored}\"")),
        "a rooted cover path must render as an image: {html}"
    );
}

#[tokio::test]
async fn a_draft_is_absent_from_the_list_and_404s_on_its_page() {
    let db = full_db().await;
    let router = router(db.clone());
    let draft = draft_post(&db).await;
    assert_eq!(draft.status, "draft", "the fixture must be a draft");

    let client = TestClient::new(&router);

    let list = client.get("/blog").await;
    assert_eq!(list.status(), 200);
    let list_html = body_string(list).await;
    assert!(
        !list_html.contains(&draft.title),
        "a draft must not appear in the list: {list_html}"
    );
    assert!(
        !list_html.contains(&draft.body),
        "a draft's body must not appear in the list: {list_html}"
    );

    let detail = client.get(&post_path(&draft)).await;
    assert_eq!(
        detail.status(),
        404,
        "a draft's page must be not found, got {}",
        detail.status()
    );
    let detail_html = body_string(detail).await;
    assert!(
        !detail_html.contains(&draft.body),
        "a draft's body must not leak through its 404: {detail_html}"
    );
}

#[tokio::test]
async fn an_id_that_names_no_post_is_not_found() {
    let db = full_db().await;
    let router = router(db);
    let client = TestClient::new(&router);

    // A well-formed id no row carries...
    let missing = client.get(&format!("/blog/{}", uuid::Uuid::new_v4())).await;
    assert_eq!(missing.status(), 404, "an unknown id must 404");

    // ...and a segment that is not an id at all: the URL names a post, so no
    // parse failure becomes a 400.
    let malformed = client.get("/blog/not-a-uuid").await;
    assert_eq!(malformed.status(), 404, "a malformed id must 404");
}

#[tokio::test]
async fn an_empty_blog_lists_nothing_without_failing() {
    // No seed at all: the page must answer, not error on an empty table.
    let db = empty_schema_db().await;
    let router = router(db);

    let response = TestClient::new(&router).get("/blog").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    assert!(
        html.contains("No posts have been published yet."),
        "an empty blog must say so: {html}"
    );
}

#[tokio::test]
async fn the_list_carries_every_published_post_and_its_author() {
    // More than one row is what makes the include load-bearing: a per-row read
    // of an un-included `Deferred` panics, and every listed author must render.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let author = Author::all()
        .first()
        .exec(&mut db_q)
        .await
        .expect("query an author")
        .expect("the seed creates authors");
    for index in 0..5u128 {
        create_published(
            &db,
            index,
            &format!("Public Post {index}"),
            &format!("Excerpt {index}"),
            OLDER,
            &author,
        )
        .await;
    }

    let response = TestClient::new(&router).get("/blog").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    for index in 0..5u128 {
        assert!(
            html.contains(&format!("Public Post {index}")),
            "the list must show every published post: {html}"
        );
        assert!(
            html.contains(&format!("Excerpt {index}")),
            "the list must show every excerpt: {html}"
        );
    }
    assert!(
        html.contains(&author.name),
        "the list must name every post's author: {html}"
    );
}

#[tokio::test]
async fn a_post_with_no_description_renders_no_empty_excerpt() {
    // The excerpt is guarded, so a published post whose SEO description is
    // empty renders no empty paragraph. Asserted structurally — an empty
    // element, not the class it would carry.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let author = Author::all()
        .first()
        .exec(&mut db_q)
        .await
        .expect("query an author")
        .expect("the seed creates authors");
    create_published(&db, 0, "Bare Post", "", OLDER, &author).await;

    let response = TestClient::new(&router).get("/blog").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    assert!(
        html.contains("Bare Post"),
        "the post must still be listed: {html}"
    );
    assert!(
        !html.contains("></p>"),
        "a post with no description must render no empty paragraph: {html}"
    );
}

#[tokio::test]
async fn the_list_orders_posts_newest_first() {
    // The order is `created_at` descending. The seed's one published post is
    // dated 2024-01-15, so two more bracket it and the whole list is pinned.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let author = Author::all()
        .first()
        .exec(&mut db_q)
        .await
        .expect("query an author")
        .expect("the seed creates authors");
    create_published(&db, 0, "Older Post", "Older excerpt", OLDER, &author).await;
    create_published(&db, 1, "Newer Post", "Newer excerpt", NEWER, &author).await;

    let response = TestClient::new(&router).get("/blog").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;

    let seeded = published_post(&db).await;
    let position = |title: &str| {
        html.find(title)
            .unwrap_or_else(|| panic!("the list must carry {title}: {html}"))
    };
    let newer = position("Newer Post");
    let older = position("Older Post");
    let seed = position(&seeded.title);
    assert!(
        newer < older && older < seed,
        "the list must read newest first: Newer Post at {newer}, Older Post at {older}, \
         {} at {seed}: {html}",
        seeded.title
    );
}

/// The blog is public, not admin-only: a session changes nothing about it.
#[tokio::test]
async fn a_signed_in_visitor_sees_the_same_blog() {
    let db = full_db().await;
    let router = router(db.clone());
    let post = published_post(&db).await;

    let anonymous = TestClient::new(&router).get("/blog").await;
    let anonymous_html = body_string(anonymous).await;
    let signed_in = demo_client(&router, &db).await.get("/blog").await;
    assert_eq!(signed_in.status(), 200);
    let signed_in_html = body_string(signed_in).await;

    assert_eq!(
        anonymous_html, signed_in_html,
        "a session must not change the public list"
    );
    let detail = demo_client(&router, &db).await.get(&post_path(&post)).await;
    assert_eq!(detail.status(), 200);
}
