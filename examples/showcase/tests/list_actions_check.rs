use showcase::{app::router_for_tests as router, models::User};

use crate::common::{body_string, demo_client, full_db, post_count, row_titles, seeded_db};

// GH #162 (Filament's List `CreateAction` + `recordActions` EditAction):
// every list exposes its create/edit entry points as real links — the live
// (`live_search`) lists included, where the table swaps in place below an
// eager header.

#[tokio::test]
async fn users_list_links_to_create_and_edit() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let resp = client.get("/admin/users").await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(
        html.contains("href=\"/admin/users/create\"") && html.contains("Create"),
        "missing Create entry point in {html}"
    );
    let mut db_check = db.clone();
    let ada = User::filter(User::fields().name().eq("Ada Lovelace".to_string()))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap()
        .expect("Ada is seeded");
    assert!(
        html.contains(&format!("href=\"/admin/users/{}/edit\"", ada.id)) && html.contains(">Edit<"),
        "missing Edit link for Ada in {html}"
    );
}

#[tokio::test]
async fn authors_list_links_to_create_and_edit() {
    let db = full_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let resp = client.get("/admin/authors").await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(
        html.contains("href=\"/admin/authors/create\"") && html.contains("Create"),
        "missing Create entry point in {html}"
    );
    assert!(
        html.contains("/admin/authors/") && html.contains("/edit") && html.contains(">Edit<"),
        "missing Edit links in {html}"
    );
}

#[tokio::test]
async fn posts_list_links_to_create_and_edit() {
    let db = full_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let resp = client.get("/admin/posts").await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(
        html.contains("href=\"/admin/posts/create\"") && html.contains("Create"),
        "missing Create entry point in {html}"
    );
    assert!(
        html.contains("/admin/posts/") && html.contains("/edit") && html.contains(">Edit<"),
        "missing Edit links in {html}"
    );
}

#[tokio::test]
async fn posts_pagination_walks_forward_and_back() {
    // GH #184: the seed carries enough posts to cross a page boundary, so the
    // pager is a walkable demo — not a control that never renders. This walks a
    // real cursor: page 1 -> after= -> page 2 -> before= -> page 1.
    let db = crate::common::full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    let total = post_count(&db).await;
    assert!(
        total > 25,
        "the fixture must exceed one page for this to mean anything, got {total}"
    );

    let html = body_string(client.get("/admin/posts").await).await;
    let first_page = row_titles(&html);
    assert_eq!(
        first_page.len(),
        25,
        "a page holds the declared page size, got {}",
        first_page.len()
    );
    assert!(
        first_page.contains(&"Hello Toasty".to_string()),
        "the seeded story must be reachable from page 1, got {first_page:?}"
    );

    let next = next_link(&html).expect("page 1 must offer a next page");
    assert!(
        next.contains("after="),
        "the forward link must carry a cursor, got {next}"
    );

    let html = body_string(client.get(&next).await).await;
    let second_page = row_titles(&html);
    assert!(
        !second_page.is_empty(),
        "following the cursor must render rows"
    );
    assert_ne!(
        first_page, second_page,
        "the second page must differ from the first"
    );
    for title in &first_page {
        assert!(
            !second_page.contains(title),
            "cursor pagination must not repeat {title:?} across pages"
        );
    }

    let back = previous_link(&html).expect("page 2 must offer a previous page");
    assert!(
        back.contains("before="),
        "the backward link must carry a cursor, got {back}"
    );
    let html = body_string(client.get(&back).await).await;
    assert_eq!(
        row_titles(&html),
        first_page,
        "walking back must restore page 1 unchanged"
    );
}

/// The `Next` pager link's href, HTML-unescaped.
fn next_link(html: &str) -> Option<String> {
    link_href(html, "Next")
}

/// The `Previous` pager link's href, HTML-unescaped.
fn previous_link(html: &str) -> Option<String> {
    link_href(html, "Previous")
}

/// Grab the `href` of the pager anchor whose accessible label is `label`.
///
/// `pagination_next` / `pagination_previous` render an `sr-only` span with the
/// direction, which is the only stable handle on the link (it has no id).
fn link_href(html: &str, label: &str) -> Option<String> {
    let at = html.find(&format!(">{label}</span>"))?;
    let before = &html[..at];
    let start = before.rfind("href=\"")?;
    let rest = &html[start + 6..];
    let end = rest.find('"')?;
    Some(rest[..end].replace("&amp;", "&"))
}
