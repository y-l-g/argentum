use showcase::app::router_for_tests as router;

use crate::common::{body_string, demo_client, find_href_with, full_db};

// GH #162 (Filament's List `CreateAction` + `recordActions` EditAction):
// every list exposes its create/edit entry points as real links — the live
// (`live_search`) lists included, where the table swaps in place below an
// eager header.
#[tokio::test]
async fn lists_link_to_create_and_edit() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    for prefix in ["/admin/users", "/admin/authors", "/admin/posts"] {
        let resp = client.get(prefix).await;
        assert!(
            resp.status().is_success(),
            "{prefix} status {}",
            resp.status()
        );
        let html = body_string(resp).await;
        assert!(
            html.contains(&format!("href=\"{prefix}/create\"")),
            "{prefix} must link its create page, got {html}"
        );
        let edit = find_href_with(&html, "/edit")
            .unwrap_or_else(|| panic!("{prefix} must link a row's edit page, got {html}"));
        assert!(
            edit.starts_with(&format!("{prefix}/")) && edit.ends_with("/edit"),
            "{prefix} edit link must address its own resource, got {edit}"
        );
    }
}
