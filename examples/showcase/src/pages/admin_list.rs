use argentum_core::{Resource, db::db};
use topcoat::{
    Result,
    context::{Cx, memoize},
    router::page,
    view::view,
};

use crate::app::UserResource;
use crate::models::User;

// ---------------------------------------------------------------------------
// Data loading (memoized, per-request)
// ---------------------------------------------------------------------------

#[memoize(as_ref)]
async fn query_users(cx: &Cx) -> Result<Vec<User>> {
    UserResource::query(cx)
        .exec(&mut db(cx))
        .await
        .map_err(Into::into)
}

async fn users(cx: &Cx) -> Result<&Vec<User>> {
    query_users(cx)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()).into())
}

// ---------------------------------------------------------------------------
// Page: real admin list (the "real app" result)
// ---------------------------------------------------------------------------

#[page("/admin")]
async fn admin_list(cx: &Cx) -> Result {
    let users = users(cx).await?;
    view! {
        <div class="ac-page">
            <h1>"Users"</h1>
            <p class="ac-muted">"Real admin list — data via Resource::query + db(cx) + #[memoize]"</p>
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
            <p><a href="/admin/showcase">"→ Showcase (all features)"</a></p>
        </div>
    }
}
