//! The `user-list` example binary: wire the database, seed it, and serve.

use toasty::Db;
use topcoat::router::{Router, RouterBuilderDiscoverExt};

use user_list::models::seed;

#[tokio::main]
async fn main() {
    let mut db = Db::builder()
        .models(toasty::models!(user_list::models::User))
        .connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");

    db.push_schema().await.expect("push schema");
    seed(&mut db).await.expect("seed users");

    let router = Router::builder().discover().app_context(db).build();

    topcoat::start(router).await.expect("serve");
}
