//! The example's single page: an admin-style user list.

use topcoat::{
    Result,
    context::Cx,
    router::page,
    view::{component, view},
};

use crate::models::users;

/// Renders the list of users served by the router's `/` page.
#[page("/")]
async fn home() -> Result {
    view! {
        <!DOCTYPE html>
        <html>
            <head><title>"Users"</title></head>
            <body>
                <h1>"Users"</h1>
                user_table()
            </body>
        </html>
    }
}

/// The user table body. Loads the request-memoized list of users and renders
/// a row per user.
#[component]
async fn user_table(cx: &Cx) -> Result {
    let rows = users(cx).await?;

    view! {
        <table>
            <thead>
                <tr>
                    <th>"Name"</th>
                    <th>"Email"</th>
                </tr>
            </thead>
            <tbody>
                for user in rows {
                    <tr data-user-id=(user.id.to_string())>
                        <td>(&user.name)</td>
                        <td>(&user.email)</td>
                    </tr>
                }
            </tbody>
        </table>
    }
}
