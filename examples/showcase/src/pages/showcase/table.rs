use argentum_core::{Resource, Table, TableSignals, TableState, TextColumn};
use topcoat::{
    Result,
    context::Cx,
    router::{error::bad_request, page},
    runtime::{Signal, shard, signal},
    view::{View, view},
};

use crate::{app::UserResource, models::User};

/// The demos render under the showcase path: bound controls keep it as their
/// no-JS `href`/form fallback while their handlers prevent the navigation.
const PATH: &str = "/admin/showcase/table";

/// The declared table for one demo variant (GH #154 §2).
///
/// `kind` arrives from the client on the shard endpoint, so an unknown value
/// answers `None` — the shard rejects it instead of falling back to a table
/// the snippet does not describe.
fn demo_table(cx: &Cx, kind: &str) -> Option<Table<User>> {
    match kind {
        "plain" => Some(
            Table::<User>::r#for(cx)
                .id(|u| u.id.to_string())
                .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone())),
        ),
        "searchable" => Some(
            Table::<User>::r#for(cx)
                .id(|u| u.id.to_string())
                .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable()),
        ),
        "sortable" => Some(
            Table::<User>::r#for(cx)
                .id(|u| u.id.to_string())
                .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable()),
        ),
        "both" => Some(
            Table::<User>::r#for(cx).id(|u| u.id.to_string()).columns((
                TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(User::fields().email(), |u| u.email.clone()),
            )),
        ),
        _ => None,
    }
}

/// One live demo table (GH #154 §2): the page owns the interaction signals
/// (`q`, `sort`, `dir`) and this shard reloads and re-renders its grid in
/// place when one of them changes — no navigation, no scroll jump.
///
/// The page-level counterpart of the resource list's `table_search` shard:
/// the table is page-owned, so the loader is [`Table::load`] over
/// `UserResource::query` (the tenancy seam stays `Resource::query`, P10), and
/// the swapped region excludes the search toolbar the page rendered above.
/// Every signal read here is client input and is re-parsed through
/// [`TableState::from_live_args`], exactly like the resource shard.
#[shard]
pub async fn table_demo(
    cx: &Cx,
    kind: String,
    q: Signal<String>,
    sort: Signal<String>,
    dir: Signal<String>,
) -> Result<impl View> {
    // Runtime endpoints bypass the page's auth gate (topcoat shard contract):
    // resolve the same gate here so a direct POST cannot read the demo rows.
    if argentum_core::auth::enforced(cx) {
        argentum_core::auth::require_authenticated(cx)?;
    }
    let table = demo_table(cx, &kind)
        .ok_or_else(|| bad_request("unknown table demo"))?
        // The page above owns the toolbar and the rows render only here, so
        // the swap payload never nests a second input.
        .search(false)
        .without_skeleton();
    let state = TableState::from_live_args(&q.get(), "", &sort.get(), &dir.get(), "");
    let page = table.load(cx, UserResource::query(cx), &state).await?;
    let signals = TableSignals {
        q: q.clone(),
        // The demos do not paginate or filter; the cursors and the filters
        // transport still need handles for the shared controls, so the shard
        // creates inert ones (a shard body keeps its signals across reruns).
        filters: signal(cx, String::new),
        sort: sort.clone(),
        dir: dir.clone(),
        after: signal(cx, String::new),
        before: signal(cx, String::new),
    };
    table
        .render_live_with_state(cx, page, &state, PATH, signals)
        .await
}

#[page("/admin/showcase/table")]
async fn table_showcase(cx: &Cx) -> Result<impl View> {
    // Every demo is live (GH #154 §2): the page owns each table's signals,
    // the `table_demo` shard re-renders its grid in place, and `?q=`/`?sort=`
    // stay accepted as the no-JS fallback the GET toolbar submits. Resource
    // `query` is the loader seam (P10) even for demos.
    let state = TableState::from_cx(cx);

    // Plain: nothing to search or sort, so it renders directly — declaration
    // and rows, no shard.
    let plain_table = demo_table(cx, "plain").expect("declared demo");
    let plain_page = plain_table
        .load(cx, UserResource::query(cx), &TableState::default())
        .await?;
    let plain_html = plain_table.render(cx, plain_page).await?;

    // Searchable: a page-owned `q` signal feeds the live input (eager) and
    // the shard (grid). Signals are plain locals because a shard argument
    // captures an identifier, not a field of a larger value.
    let searchable_q = signal(cx, || state.search.clone().unwrap_or_default());
    let searchable_sort = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| s.column.clone())
            .unwrap_or_default()
    });
    let searchable_dir = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" })
            .unwrap_or("asc")
            .to_string()
    });
    let searchable_signals = TableSignals {
        q: searchable_q.clone(),
        filters: signal(cx, String::new),
        sort: searchable_sort.clone(),
        dir: searchable_dir.clone(),
        after: signal(cx, String::new),
        before: signal(cx, String::new),
    };
    let searchable_table = demo_table(cx, "searchable").expect("declared demo");
    let searchable_host = searchable_table
        .render_live_search_bar(cx, &state, PATH, &searchable_signals)
        .await?;

    // Sortable: only the header changes the rows; `q` stays inert.
    let sortable_q = signal(cx, || state.search.clone().unwrap_or_default());
    let sortable_sort = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| s.column.clone())
            .unwrap_or_default()
    });
    let sortable_dir = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" })
            .unwrap_or("asc")
            .to_string()
    });

    // Composition: the same seam drives search and sort together.
    let both_q = signal(cx, || state.search.clone().unwrap_or_default());
    let both_sort = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| s.column.clone())
            .unwrap_or_default()
    });
    let both_dir = signal(cx, || {
        state
            .sort
            .as_ref()
            .map(|s| if s.descending { "desc" } else { "asc" })
            .unwrap_or("asc")
            .to_string()
    });
    let both_signals = TableSignals {
        q: both_q.clone(),
        filters: signal(cx, String::new),
        sort: both_sort.clone(),
        dir: both_dir.clone(),
        after: signal(cx, String::new),
        before: signal(cx, String::new),
    };
    let both_table = demo_table(cx, "both").expect("declared demo");
    let both_host = both_table
        .render_live_search_bar(cx, &state, PATH, &both_signals)
        .await?;

    // Demonstrate Table owns query — OR across searchable, first sortable
    let demo_search = Table::<User>::r#for(cx).columns((
        TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable(),
        TextColumn::r#for(User::fields().email(), |u| u.email.clone()).searchable(),
    ));
    let _search_expr = demo_search.search_expr("Ada"); // OR
    let _order_by = Table::<User>::r#for(cx)
        .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable())
        .order_by(false);

    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Table")
                argentum_ui::page_description(
                    "Declarative list view — columns declare how to query (searchable → starts_with, sortable → order_by) and how to render. Table owns the query. Every demo below is live: searching filters its rows and clicking a sortable header sorts, both morphing in place — no navigation, no scroll jump. /admin/users is the same seam over a Resource."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "TextColumn — plain"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (plain_html)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "searchable"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Marks column for global search → `starts_with` (portable). Table ORs across searchable columns. Type below to filter the rows in place."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).searchable());\ntable.search_expr(\"Ada\") // → starts_with OR"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (searchable_host)
                    table_demo(
                        kind: "searchable".to_owned(),
                        q: $(searchable_q),
                        sort: $(searchable_sort),
                        dir: $(searchable_dir)
                    )
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "sortable"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Marks column for ordering → `asc()` with PK tie-breaker via `order_bys()` for deterministic pagination. Click the header to sort; click again to flip direction."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).sortable());\ntable.order_by() // → asc\ntable.order_bys() // → [sortable asc, pk asc]"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    table_demo(
                        kind: "sortable".to_owned(),
                        q: $(sortable_q),
                        sort: $(sortable_sort),
                        dir: $(sortable_dir)
                    )
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Composition (tuple)"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Tuples of columns compose: one table searches and sorts, each interaction re-rendering its rows in place."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Table::for::<User>(cx).columns((\n    TextColumn::for(User::fields().name()).searchable().sortable(),\n    TextColumn::for(User::fields().email()),\n))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (both_host)
                    table_demo(
                        kind: "both".to_owned(),
                        q: $(both_q),
                        sort: $(both_sort),
                        dir: $(both_dir)
                    )
                </div>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        )
    })
}
