use argentum_core::{Panel, Resource, Schema, Table, TextColumn, TextInput};
use toasty::Db;
use topcoat::{
    Result,
    context::Cx,
    router::{Router, layout},
    view::view,
};

use crate::models::User;

// ---------------------------------------------------------------------------
// Resource — single Model → Resource, see CONTEXT.md
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
///
/// Manual `Resource` impl — `#[derive(Resource)]` currently only supports
/// `model`/`query`, not `table`. Slice 3 needs a custom `Table` so we
/// implement `Resource` by hand; a `#[resource(table=...)]` derive extension
/// will replace this in a later slice.
pub struct UserResource;

impl Resource for UserResource {
    type Model = User;

    fn table(cx: &Cx) -> Table<User> {
        Table::r#for(cx).columns((
            TextColumn::r#for(User::fields().name())
                .searchable()
                .sortable(),
            TextColumn::r#for(User::fields().email()),
        ))
    }

    fn form(_cx: &Cx) -> Schema {
        // Canonical Resource::form seam (spec #6 solution) — typed lens → TextInput.
        // Proves both Resource entry points are wired; showcase pages use this
        // indirectly via Schema::new, but resource owners declare forms here.
        Schema::new((
            TextInput::r#for(User::fields().name()).required(),
            TextInput::r#for(User::fields().email()).required().email(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Layout — Panel shell at /admin, wraps every /admin/* page
// ---------------------------------------------------------------------------

#[layout("/admin")]
async fn admin_layout(slot: Result) -> Result {
    let nav = UserResource::navigation();
    let label = nav.label.clone();
    let url = nav.url.clone();
    view! {
        <!DOCTYPE html>
        <html>
            <head><title>"Admin"</title></head>
            <body class="ac-admin">
                <nav class="ac-sidebar">
                    <a href=(url) class="ac-nav-item">(label)</a>
                    <a href="/admin/showcase" class="ac-nav-item">"Showcase"</a>
                </nav>
                <main class="ac-main">(slot?)</main>
            </body>
        </html>
    }
}

// ---------------------------------------------------------------------------
// Router helper (used by `main.rs` and tests)
// ---------------------------------------------------------------------------

pub fn router(db: Db) -> Router {
    Panel::new("admin").app_context(db).build()
}
