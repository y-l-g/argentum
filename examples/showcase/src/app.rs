use argentum_core::{Panel, Resource};
use toasty::Db;
use topcoat::{
    Result,
    router::{Router, layout},
    view::view,
};

// ---------------------------------------------------------------------------
// Resource — single Model → Resource, see CONTEXT.md
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
#[derive(Resource)]
#[resource(model = crate::models::User)]
pub struct UserResource;

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
