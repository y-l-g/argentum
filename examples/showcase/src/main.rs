use toasty::Db;

use showcase::{
    app::router,
    models::{seed, seed_phase2},
};

#[tokio::main]
async fn main() {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect to in-memory sqlite");

    db.push_schema().await.expect("push schema");
    seed(&mut db).await.expect("seed users");
    seed_phase2(&mut db).await.expect("seed phase2");

    let router = router(db);

    topcoat::start(router).await.expect("serve");
}
