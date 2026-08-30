use argentum_core::{Resource, TablePage, TableState, db::db};
use topcoat::{
    Result,
    context::{Cx, memoize},
    router::page,
    view::view,
};

use crate::{app::UserResource, models::User};

// ---------------------------------------------------------------------------
// Memoized loader — q/sort/cursor-aware (the non-shard half of GH #13; the
// boundary/shard migration itself stays behind ADR-0003)
// ---------------------------------------------------------------------------

#[memoize(as_ref)]
async fn load_users(cx: &Cx, state: &TableState) -> Result<TablePage<User>> {
    let table = UserResource::table(cx);
    let mut query = UserResource::query(cx);
    if let Some(term) = &state.search
        && let Some(expr) = table.search_expr(term)
    {
        query = query.filter(expr);
    }
    for ord in table.order_bys_for_state(state) {
        query = query.order_by(ord);
    }
    let mut db = db(cx);
    match table.page_size() {
        Some(per_page) => {
            let mut paginated = query.paginate(per_page);
            if let Some(cursor) = &state.after {
                paginated = paginated.after(argentum_core::decode_cursor(cursor)?);
            } else if let Some(cursor) = &state.before {
                paginated = paginated.before(argentum_core::decode_cursor(cursor)?);
            }
            let page: toasty::stmt::Page<User> = paginated
                .exec(&mut db)
                .await
                .map_err(topcoat::Error::from)?;
            TablePage::from_toasty_page(page)
        }
        None => {
            let rows: Vec<User> = query.exec(&mut db).await.map_err(topcoat::Error::from)?;
            Ok(rows.into())
        }
    }
}

async fn users_page<'a>(cx: &'a Cx, state: &'a TableState) -> Result<&'a TablePage<User>> {
    // `#[memoize(as_ref)]` caches `Result<&T, &Error>`; `topcoat::Error`
    // (anyhow) is not Clone, so the borrowed error must be re-wrapped into an
    // owned one here — stringification is the sanctioned workaround at this
    // boundary (EXTERNAL_GAPS.md "Topcoat — memoize(as_ref) error conversion").
    // Do not spread the pattern to non-memoized call sites.
    load_users(cx, state)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()).into())
}

// ---------------------------------------------------------------------------
// Page: the real admin list — search box, sort links, cursor pagination
// ---------------------------------------------------------------------------

#[page("/admin")]
async fn admin_list(cx: &Cx) -> Result {
    let state = TableState::from_cx(cx);
    let page = users_page(cx, &state).await?;
    let table = UserResource::table(cx);
    let table_view = table.render(cx, page).await?;
    view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Users")
                argentum_ui::page_description(
                    "Real admin list — search in the box, sort via the column links, page via Previous/Next. State lives in the URL: ?q=, ?sort=, ?dir=, ?after=, ?before=."
                )
            )
            argentum_ui::page_content((table_view))
            argentum_ui::page_content(
                <p>
                    <a
                        href="/admin/showcase"
                        class="text-sm text-primary hover:underline"
                    >
                        "→ Showcase (all features)"
                    </a>
                </p>
            )
        )
    }
}
