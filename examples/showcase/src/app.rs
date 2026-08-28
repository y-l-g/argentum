use argentum_core::{Panel, Resource, Schema, Table, TextColumn, TextInput};
use toasty::Db;
use topcoat::{
    Result,
    asset::Asset,
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{Router, layout},
    tailwind,
    view::view,
};

/// The theme's sans font, pulled from Fontsource and self-hosted as a Topcoat asset.
const GEIST: Font = fontsource_font!(GEIST, host: Asset);

use crate::models::User;

// ---------------------------------------------------------------------------
// Resource — single Model → Resource, see CONTEXT.md
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
///
/// Manual `Resource` impl — `#[derive(Resource)]` currently only supports
/// `model`/`query`, not `table`. A custom `Table` is needed so we implement
/// `Resource` by hand; a `#[resource(table=...)]` derive extension will
/// replace this later.
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
async fn admin_layout(cx: &Cx, slot: Result) -> Result {
    // Panel-aware navigation — URL respects the Panel mount prefix (e.g. "backoffice" → "/backoffice").
    let nav = Panel::new("admin").navigation_item::<UserResource>();
    let label = nav.label.clone();
    let url = nav.url.clone();
    // Asset-dependent links (tailwind + font) require `AssetConfig` on the router.
    // In `cargo test` without `AssetBundle`, we fall back to a plain style tag.
    let has_assets = topcoat::context::try_app_context::<topcoat::asset::AssetConfig>(cx).is_some();
    view! {
        cx =>
        <!DOCTYPE html>
        <html>
            <head>
                <title>"Admin"</title>
                topcoat::dev::script()
                if has_assets {
                    topcoat::font::link(font: GEIST)
                    <link rel="stylesheet" href=(tailwind::stylesheet!())>
                } else {
                    <style>"/* tailwind and font skipped - no asset config */"</style>
                }
            </head>
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
