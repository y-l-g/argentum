use argentum_core::{Resource, Table, TextColumn, db::db};
use topcoat::{Result, context::Cx, router::page, view::view};

use crate::{app::UserResource, models::User};

#[page("/admin/showcase/table")]
async fn table_showcase(cx: &Cx) -> Result {
    // Respect Resource::query seam (CONTEXT Query) — every loader starts
    // from Resource::query, not Model::all, so tenancy/soft-delete scoping
    // applies even in showcase demos (P10).
    let mut conn = db(cx);
    let rows = UserResource::query(cx)
        .exec(&mut conn)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Variants for snippet display
    let plain = Table::<User>::r#for(cx).columns(TextColumn::r#for(User::fields().name()));
    let searchable =
        Table::<User>::r#for(cx).columns(TextColumn::r#for(User::fields().name()).searchable());
    let sortable =
        Table::<User>::r#for(cx).columns(TextColumn::r#for(User::fields().name()).sortable());
    let both = Table::<User>::r#for(cx).columns((
        TextColumn::r#for(User::fields().name())
            .searchable()
            .sortable(),
        TextColumn::r#for(User::fields().email()),
    ));

    let plain_html = plain.render(cx, &rows).await?;
    let searchable_html = searchable.render(cx, &rows).await?;
    let sortable_html = sortable.render(cx, &rows).await?;
    let both_html = both.render(cx, &rows).await?;

    // Demonstrate Table owns query — OR across searchable, first sortable
    let demo_search = Table::<User>::r#for(cx).columns((
        TextColumn::r#for(User::fields().name()).searchable(),
        TextColumn::r#for(User::fields().email()).searchable(),
    ));
    let _search_expr = demo_search.search_expr("Ada"); // OR
    let _order_by = Table::<User>::r#for(cx)
        .columns(TextColumn::r#for(User::fields().name()).sortable())
        .order_by();

    view! {
        <div class="ac-showcase">
            <h1>"Table"</h1>
            <p>"Declarative list view — columns declare how to query (searchable → starts_with, sortable → order_by) and how to render. Table owns the query."</p>

            <section class="ac-showcase-block">
                <h2>"TextColumn — plain"</h2>
                <pre><code>"Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()))"</code></pre>
                <div class="ac-showcase-result">(plain_html)</div>
            </section>

            <section class="ac-showcase-block">
                <h2>"searchable"</h2>
                <p>"Marks column for global search → `starts_with` (portable). Table ORs across searchable columns."</p>
                <pre><code>"let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).searchable());\ntable.search_expr(\"Ada\") // → starts_with OR"</code></pre>
                <div class="ac-showcase-result">(searchable_html)</div>
            </section>

            <section class="ac-showcase-block">
                <h2>"sortable"</h2>
                <p>"Marks column for ordering → `asc()` with PK tie-breaker via `order_bys()` for deterministic pagination."</p>
                <pre><code>"let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).sortable());\ntable.order_by() // → asc\ntable.order_bys() // → [sortable asc, pk asc]"</code></pre>
                <div class="ac-showcase-result">(sortable_html)</div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Composition (tuple)"</h2>
                <pre><code>"Table::for::<User>(cx).columns((\n    TextColumn::for(User::fields().name()).searchable().sortable(),\n    TextColumn::for(User::fields().email()),\n))"</code></pre>
                <div class="ac-showcase-result">(both_html)</div>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        </div>
    }
}
