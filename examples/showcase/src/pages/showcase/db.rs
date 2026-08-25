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
// Memoized loader — the pattern every page/shard uses (from user-list example)
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

#[page("/admin/showcase/db")]
async fn db_showcase(cx: &Cx) -> Result {
    let rows = users(cx).await?;
    let count = rows.len();

    view! {
        <div class="ac-showcase">
            <h1>"Db + memoize"</h1>
            <p>"Db lives in app_context (Arc-pooled). Clone per request, exec with &mut. #[memoize] dedups per request and shares one future across concurrent callers."</p>

            <section class="ac-showcase-block">
                <h2>"db(cx) glue"</h2>
                <pre><code>"use argentum_core::db::db;\n// in page/shard/procedure:\nlet mut db = db(cx); // app_context::<Db>(cx).clone()\nlet rows = User::all().exec(&mut db).await?;"</code></pre>
                <div class="ac-showcase-result">
                    <p>"Rows from memoized loader: " (count.to_string())</p>
                    <ul>
                        for user in rows {
                            <li>(format!("{} — {}", user.name, user.email))</li>
                        }
                    </ul>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"#[memoize] loader — variants"</h2>
                <pre><code>"#[memoize(as_ref)]\nasync fn query_users(cx: &Cx) -> Result<Vec<User>> {\n    UserResource::query(cx).exec(&mut db(cx)).await.map_err(Into::into)\n}\nasync fn users(cx: &Cx) -> Result<&Vec<User>> {\n    query_users(cx).await.map_err(|e| std::io::Error::other(e.to_string()).into())\n}\n// concurrent: 8 tasks calling users(&cx) → body runs once (AtomicUsize == 1)"</code></pre>
                <p class="ac-muted">"This is the same pattern as the former user-list example, now consolidated here to avoid duplication. The dedup guarantee is exercised by the integration test and by concurrent page fragments."</p>
            </section>

            <section class="ac-showcase-block">
                <h2>"Why clone?"</h2>
                <pre><code>"// Pool: num_cpus*2, health-sweep, pre_ping (toasty/src/db/pool.rs)\n// exec needs &mut Db → clone per request is a cheap Arc bump"</code></pre>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        </div>
    }
}
