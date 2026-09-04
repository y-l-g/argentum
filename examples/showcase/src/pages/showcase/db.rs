use argentum_core::{Resource, db::db};
use topcoat::{
    Result,
    context::{Cx, memoize},
    router::page,
    view::{View, view},
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
    // #[memoize(as_ref)] caches Result<&T, &Error>, and `topcoat::Error`
    // (anyhow) is not Clone, so the borrowed error must be converted into an
    // owned one here — stringification is the workaround. See EXTERNAL_GAPS.md
    // "Topcoat — memoize(as_ref) error conversion". Use `.map_err(Into::into)`
    // in non-memoized loaders; do not spread this pattern.
    query_users(cx)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()).into())
}

#[page("/admin/showcase/db")]
async fn db_showcase(cx: &Cx) -> Result<impl View> {
    let rows = users(cx).await?;
    let count = rows.len();

    Ok(view! {
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Db + memoize")
                argentum_ui::page_description(
                    "Db lives in app_context (Arc-pooled). Clone per request, exec with &mut. #[memoize] dedups per request and shares one future across concurrent callers."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "db(cx) glue"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "use argentum_core::db::db;\n// in page/shard/procedure:\nlet mut db = db(cx); // app_context::<Db>(cx).clone()\nlet rows = User::all().exec(&mut db).await?;"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">
                        "Rows from memoized loader: "
                        (count.to_string())
                    </p>
                    <ul>
                        for user in rows {
                            <li>(format!("{} — {}", user.name, user.email))</li>
                        }
                    </ul>
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "#[memoize] loader — variants"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "#[memoize(as_ref)]\nasync fn query_users(cx: &Cx) -> Result<Vec<User>> {\n    UserResource::query(cx).exec(&mut db(cx)).await.map_err(Into::into)\n}\nasync fn users(cx: &Cx) -> Result<&Vec<User>> {\n    query_users(cx).await.map_err(|e| std::io::Error::other(e.to_string()).into())\n}\n// concurrent: 8 tasks calling users(&cx) → body runs once (AtomicUsize == 1)\n// NOTE: stringify only needed because memoize(as_ref) caches &Error; normal pages use `?` or map_err(Into::into) without stringify"
                )
                <p class="text-sm text-muted-foreground">
                    "This is the same pattern as the former user-list example, now consolidated here to avoid duplication. The dedup guarantee is exercised by the integration test and by concurrent page fragments."
                </p>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Why clone?"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "// Pool: num_cpus*2, health-sweep, pre_ping (toasty/src/db/pool.rs)\n// exec needs &mut Db → clone per request is a cheap Arc bump"
                )
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        )
    })
}
