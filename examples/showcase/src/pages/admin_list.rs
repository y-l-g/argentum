use argentum_core::{Resource, db::db};
use topcoat::{
    Result,
    context::Cx,
    router::{page, query_params},
    view::view,
};

use crate::app::UserResource;

#[query_params]
struct AdminQuery {
    q: Option<String>,
}

// ---------------------------------------------------------------------------
// Page: real admin list (the "real app" result) — now via Table
// ---------------------------------------------------------------------------

#[page("/admin")]
async fn admin_list(cx: &Cx) -> Result {
    // Plain `q` param for this slice — parse errors treated as empty (no filter) to keep minimal.
    // Review P2: q-aware `#[memoize]` is deferred to shard/boundary slice
    // (ADR-0003) — memoizing `db(cx)` query with `q` as key will be added
    // when search/sort move to owned signal + boundary (spec #6).
    let q = query_params::<AdminQuery>(cx)
        .ok()
        .and_then(|x| x.q.clone())
        .unwrap_or_default();
    let table = UserResource::table(cx);
    let mut query = UserResource::query(cx);
    if let Some(expr) = table.search_expr(&q) {
        query = query.filter(expr);
    }
    for ord in table.order_bys() {
        query = query.order_by(ord);
    }
    let mut db = db(cx);
    let rows = query.exec(&mut db).await?;
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
