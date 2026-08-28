use argentum_core::{NavigationItem, Panel, Resource, Schema, Table, TextColumn, TextInput};
use toasty::Db;
use topcoat::{
    Result,
    asset::{Asset, AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{RouterBuilderDiscoverExt, layout},
    tailwind,
    view::view,
};

use crate::models::User;

/// The theme's sans font, pulled from Fontsource and self-hosted as a Topcoat asset.
const GEIST: Font = fontsource_font!(GEIST, host: Asset);

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
    let nav_items = vec![
        Panel::new("admin").navigation_item::<UserResource>(),
        NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
        },
    ];
    let current = topcoat::router::request::uri(cx).path().to_string();
    let inner = slot?;
    let shell = Panel::render_shell(cx, nav_items, &current, inner, None).await?;
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
                    topcoat::runtime::script()
                    topcoat::font::link(font: GEIST)
                    <link rel="stylesheet" href=(tailwind::stylesheet!())>
                } else {
                    // Fallback for tests / offline builds without AssetBundle
                    <style>"/* tailwind and font skipped - no asset config */"</style>
                }
            </head>
            <body>
                (shell)
            </body>
        </html>
    }
}

// ---------------------------------------------------------------------------
// Router helper (used by `main.rs` and tests)
// ---------------------------------------------------------------------------

pub fn router(db: Db) -> topcoat::router::Router {
    // Include AssetBundle so tailwind stylesheet and Geist font assets are served.
    // In tests, AssetBundle may be absent (offline build), so fall back to no assets
    // while still keeping the shell's Token classes.
    match AssetBundle::load() {
        Ok(bundle) => topcoat::router::Router::builder()
            .discover()
            .app_context(db)
            .assets(bundle)
            .build(),
        Err(_) => topcoat::router::Router::builder()
            .discover()
            .app_context(db)
            .build(),
    }
}
