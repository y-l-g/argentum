use argentum_core::{Resource, Table, TextColumn, db::db};
use topcoat::{Result, context::Cx, router::page, view::view};

use crate::{app::UserResource, models::User};

#[page("/admin/showcase/table")]
async fn table_showcase(cx: &Cx) -> Result {
    // Respect Resource::query seam (CONTEXT Query) — every loader starts
    // from Resource::query, not Model::all, so tenancy/soft-delete scoping
    // applies even in showcase demos (P10).
    let mut conn = db(cx);
    let rows = UserResource::query(cx).exec(&mut conn).await?;

    // Variants for snippet display
    let plain = Table::<User>::r#for(cx)
        .id(|u| u.id.to_string())
        .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
    let searchable = Table::<User>::r#for(cx)
        .id(|u| u.id.to_string())
        .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable());
    let sortable = Table::<User>::r#for(cx)
        .id(|u| u.id.to_string())
        .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable());
    let both = Table::<User>::r#for(cx).id(|u| u.id.to_string()).columns((
        TextColumn::r#for(User::fields().name(), |u| u.name.clone())
            .searchable()
            .sortable(),
        TextColumn::r#for(User::fields().email(), |u| u.email.clone()),
    ));
    let plain_html = plain.render(cx, &rows).await?;
    let searchable_html = searchable.render(cx, &rows).await?;
    let sortable_html = sortable.render(cx, &rows).await?;
    let both_html = both.render(cx, &rows).await?;

    // Demonstrate Table owns query — OR across searchable, first sortable
    let demo_search = Table::<User>::r#for(cx).columns((
        TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable(),
        TextColumn::r#for(User::fields().email(), |u| u.email.clone()).searchable(),
    ));
    let _search_expr = demo_search.search_expr("Ada"); // OR
    let _order_by = Table::<User>::r#for(cx)
        .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable())
        .order_by(false);

    view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Table")
                argentum_ui::page_description(
                    "Declarative list view — columns declare how to query (searchable → starts_with, sortable → order_by) and how to render. Table owns the query."
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
                    "Marks column for global search → `starts_with` (portable). Table ORs across searchable columns."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).searchable());\ntable.search_expr(\"Ada\") // → starts_with OR"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (searchable_html)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "sortable"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Marks column for ordering → `asc()` with PK tie-breaker via `order_bys()` for deterministic pagination."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "let table = Table::for::<User>(cx).columns(TextColumn::for(User::fields().name()).sortable());\ntable.order_by() // → asc\ntable.order_bys() // → [sortable asc, pk asc]"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (sortable_html)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Composition (tuple)"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Table::for::<User>(cx).columns((\n    TextColumn::for(User::fields().name()).searchable().sortable(),\n    TextColumn::for(User::fields().email()),\n))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (both_html)
                </div>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        )
    }
}
