use toasty::Db;

use admin::{app::router, models::seed};

#[tokio::main]
async fn main() {
    let mut db = Db::builder()
        .models(toasty::models!(admin::models::User))
        .connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");

    db.push_schema().await.expect("push schema");
    seed(&mut db).await.expect("seed users");

    let router = router(db);

    topcoat::start(router).await.expect("serve");
}
