use showcase::{app::router_for_tests as router, models::User};

use crate::common::{body_string, demo_client, full_db, seeded_db};

// GH #162 (Filament's List `CreateAction` + `recordActions` EditAction):
// every list exposes its create/edit entry points as real links — the live
// (`live_search`) lists included, where the grid swaps in place below an
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
