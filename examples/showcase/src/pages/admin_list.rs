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
    // Plain `q` param for now — parse errors treated as empty (no filter) to keep minimal.
    // q-aware #[memoize] is deferred until search/sort move to owned signal +
    // boundary (see ADR-0003, GH #13).
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
        <div class="flex flex-col gap-6">
            <h1 class="text-2xl font-bold tracking-tight text-foreground">"Users"</h1>
            <p class="text-sm text-muted-foreground">"Real admin list — Table via Resource::table + db(cx) (q=" (q.clone()) ")"</p>
            (table_view)
            <p><a href="/admin/showcase" class="text-sm text-primary hover:underline">"→ Showcase (all features)"</a></p>
        </div>
    }
}
