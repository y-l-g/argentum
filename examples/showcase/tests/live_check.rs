//! The dashboard's live feed: a panel write reaches an open page, and the page
//! marks the WebSocket connection Topcoat opens for the feed's shard.
//!
//! The connection itself is the browser runtime's; what a server-side test can
//! pin is the markup and the wiring behind it — the page renders the rows, the
//! shard asks for the connection (and not the page around it), and a committed
//! write shows up in the feed.

use showcase::{app::router_for_tests as router, live::LIVE_PATH};

use crate::common::{TestClient, body_string, demo_client, seeded_db, user_count};

/// Create one user through the panel's own form, as an admin would.
///
/// Returns the name, which the feed renders.
async fn create_user(client: &TestClient<'_>) -> String {
    let name = format!("Live-{}", uuid::Uuid::new_v4());
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name={name}&email={name}@example.com&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        303,
        "a completed create redirects to the list"
    );
    name
}

/// The feed's slice of the page: everything after the region's marker.
fn feed(html: &str) -> &str {
    let at = html
        .find("data-live-feed")
        .unwrap_or_else(|| panic!("the feed region, got {html}"));
    &html[at..]
}

/// The dashboard page renders the feed and asks for a connection on the shard,
/// not the page.
#[tokio::test]
async fn the_dashboard_renders_the_feed_and_requests_a_shard_connection() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get(LIVE_PATH).await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "GET {LIVE_PATH} renders the panel page, got {status}"
    );
    assert!(html.contains(">Dashboard</h1>"), "the page heading: {html}");
    assert!(html.contains("data-live-feed"), "the feed region: {html}");

    // `connected(cx)` marks the render that needs a connection. The marker sits
    // inside the feed shard's own markers, so the requirement belongs to the
    // shard and not to the page around it (topcoat#445).
    let start = html
        .find("::topcoat::shard::start(")
        .unwrap_or_else(|| panic!("the feed shard's start marker, got {html}"));
    let end = html
        .find("::topcoat::shard::end(")
        .unwrap_or_else(|| panic!("the feed shard's end marker, got {html}"));
    let connect = html
        .find("::topcoat::connect")
        .unwrap_or_else(|| panic!("connected(cx) must ask for a connection, got {html}"));
    assert!(
        start < connect && connect < end,
        "the connection requirement must belong to the feed shard, got {html}"
    );
    assert_eq!(
        html.matches("::topcoat::connect").count(),
        1,
        "the page itself must request no connection: {html}"
    );
}

/// A write the panel commits shows up in the feed.
#[tokio::test]
async fn a_committed_write_shows_up_in_the_feed() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let before = user_count(&db).await;
    let name = create_user(&client).await;
    assert_eq!(
        user_count(&db).await,
        before + 1,
        "the create must commit a row"
    );

    let html = body_string(client.get(LIVE_PATH).await).await;
    assert!(
        feed(&html).contains(&name),
        "the committed write must show up in the feed: {html}"
    );
}
