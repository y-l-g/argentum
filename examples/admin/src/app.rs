use argentum_core::{Panel, Resource};
use toasty::Db;
use topcoat::{
    Result,
    context::{Cx, memoize},
    router::{Router, layout, page},
    view::view,
};

use crate::models::User;
use argentum_core::db::db;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
#[derive(Resource)]
#[resource(model = crate::models::User)]
pub struct UserResource;

// ---------------------------------------------------------------------------
// Data loading (memoized)
// ---------------------------------------------------------------------------

#[memoize(as_ref)]
async fn query_users(cx: &Cx) -> Result<Vec<User>> {
    UserResource::query(cx).exec(&mut db(cx)).await.map_err(Into::into)
}

async fn users(cx: &Cx) -> Result<&Vec<User>> {
    query_users(cx)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()).into())
}

// ---------------------------------------------------------------------------
// Layout + Page
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
                </nav>
                <main class="ac-main">(slot?)</main>
            </body>
        </html>
    }
}

#[page("/admin")]
async fn admin_list(cx: &Cx) -> Result {
    let users = users(cx).await?;
    view! {
        <h1>"Users"</h1>
        <table class="ac-table">
            <thead>
                <tr>
                    <th>"Name"</th>
                    <th>"Email"</th>
                </tr>
            </thead>
            <tbody>
                for user in users {
                    <tr>
                        <td>(&user.name)</td>
                        <td>(&user.email)</td>
                    </tr>
                }
            </tbody>
        </table>
    }
}

// ---------------------------------------------------------------------------
// Router helper (used by `main.rs` and tests)
// ---------------------------------------------------------------------------

pub fn router(db: Db) -> Router {
    Panel::new("admin").app_context(db).build()
}
