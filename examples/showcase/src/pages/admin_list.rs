use argentum_core::{Resource, db::db};
use topcoat::{
    Result,
    context::Cx,
    router::{page, query_params},
    view::view,
};

use crate::app::UserResource;
use crate::models::User;

#[query_params]
struct AdminQuery {
    q: Option<String>,
}

// ---------------------------------------------------------------------------
// Page: real admin list (the "real app" result) — now via Table
// ---------------------------------------------------------------------------

#[page("/admin")]
async fn admin_list(cx: &Cx) -> Result {
    // Plain `q` param for this slice — parse errors treated as empty (no filter) to keep minimal
    let q = query_params::<AdminQuery>(cx)
        .ok()
        .and_then(|x| x.q.clone())
        .unwrap_or_default();
    let table = UserResource::table(cx);
    let mut query = UserResource::query(cx);
    if let Some(expr) = table.search_expr(&q) {
        query = query.filter(expr);
    }
    if let Some(order) = table.order_by() {
        // PK tie-breaker for deterministic sort (pagination stability)
        query = query.order_by(order).order_by(User::fields().id().asc());
    }
    let mut db = db(cx);
    let rows = query
        .exec(&mut db)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let table_view = table.render(cx, &rows).await?;
    view! {
        <div class="ac-page">
            <h1>"Users"</h1>
            <p class="ac-muted">"Real admin list — Table via Resource::table + db(cx) (q=" (q.clone()) ")"</p>
            (table_view)
            <p><a href="/admin/showcase">"→ Showcase (all features)"</a></p>
        </div>
    }
}
