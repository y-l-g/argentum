use std::time::Instant;

use argentum_core::{Panel, Resource, Schema, Table, TableState, Tenant, TextColumn, TextInput};
use jiff::Timestamp;
use toasty::{Db, Deferred};
use topcoat::{
    Result,
    context::{Cx, CxTestBuilder},
    router::{Body, Next, Router, Slot, layer, layout, response::Response},
    view::{View, ViewExt},
};

/// Author for bench — tenant_id + posts HasMany (mirrors showcase).
#[derive(Debug, Clone, toasty::Model)]
pub struct Author {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub tenant_id: uuid::Uuid,
    pub name: String,
    #[unique]
    pub email: String,
    #[has_many]
    pub posts: Deferred<Vec<Post>>,
}

#[derive(Debug, Clone, toasty::Model)]
pub struct Post {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    #[index]
    pub tenant_id: uuid::Uuid,
    #[index]
    pub title: String,
    pub body: String,
    #[index]
    pub status: String,
    pub featured: bool,
    pub created_at: Timestamp,
    pub image_path: String,
    pub tags: String,
    #[index]
    pub author_id: uuid::Uuid,
    #[belongs_to(key = author_id, references = id)]
    pub author: Deferred<Author>,
    #[has_many]
    pub comments: Deferred<Vec<Comment>>,
}

#[derive(Debug, Clone, toasty::Model)]
pub struct Comment {
    #[key]
    #[auto]
    pub id: uuid::Uuid,
    pub body: String,
    #[index]
    pub post_id: uuid::Uuid,
    #[belongs_to(key = post_id, references = id)]
    pub post: Deferred<Post>,
}

pub struct AuthorResource;
impl Resource for AuthorResource {
    type Model = Author;
    /// Gated like the showcase's `AuthorResource` (GH #223): the harness mirrors
    /// the shipped resources, and a hand-written `tenant_id` filter here would
    /// teach the recipe the framework removed. Nothing loads through this
    /// resource — the workload is the posts list — so `query` stays the default.
    fn requires_tenant() -> bool {
        true
    }
    fn table(cx: &Cx) -> Table<Author> {
        Table::r#for(cx)
            .id(|a: &Author| a.id.to_string())
            .pk(|a: &Author| a.id.to_string())
            .columns((
                TextColumn::r#for(Author::fields().name(), |a: &Author| a.name.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(Author::fields().email(), |a: &Author| a.email.clone()),
            ))
            .paginate(2)
    }
    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            TextInput::r#for(Author::fields().name()).required(),
            TextInput::r#for(Author::fields().email())
                .required()
                .email()
                .unique(),
        ))
    }
}

pub struct PostResource;
impl Resource for PostResource {
    type Model = Post;
    // Bench policy (GH #171): the shipped list 403s unless `can_view_any`
    // passes and a tenant is present — the harness asserts both per iteration
    // so the measured path is the enforced one, not an open query.
    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    fn can_view(_cx: &Cx, _record: &Post) -> bool {
        true
    }
    // GH #226: `editable()`/`deletable()` are opt-in, so the predicates they
    // promise are declared beside them. `bench_list_path` reads only the two
    // flags below — these predicates keep the fixture honest about the pairing,
    // they do not change what is measured.
    fn can_update(_cx: &Cx, _record: &Post) -> bool {
        true
    }
    fn can_delete(_cx: &Cx, _record: &Post) -> bool {
        true
    }
    fn editable() -> bool {
        true
    }
    fn deletable() -> bool {
        true
    }
    fn requires_tenant() -> bool {
        true
    }
    /// Includes only (GH #223): the tenant filter is the framework's now —
    /// `scoped_query` derives it from `Post`'s own `tenant_id` column and ANDs
    /// it onto this — so the harness must load through `scoped_query` to
    /// measure the shipped, scoped path.
    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        let inc_author: toasty::stmt::Include<Post, Author> = Post::fields().author().into();
        let inc_comments: toasty::stmt::Include<Post, toasty::stmt::List<Comment>> =
            Post::fields().comments().into();
        toasty::stmt::Query::<toasty::stmt::List<Post>>::all()
            .include(inc_author)
            .include(inc_comments)
    }
    fn table(cx: &Cx) -> Table<Post> {
        Table::r#for(cx)
            .id(|p: &Post| p.id.to_string())
            .pk(|p: &Post| p.id.to_string())
            .columns((
                TextColumn::r#for(Post::fields().title(), |p: &Post| p.title.clone())
                    .searchable()
                    .sortable(),
                TextColumn::computed("Author", |p: &Post| {
                    if p.author.is_unloaded() {
                        "-".to_string()
                    } else {
                        p.author.get().name.clone()
                    }
                })
                .needs(["author"]),
                TextColumn::computed("Comments", |p: &Post| {
                    if p.comments.is_unloaded() {
                        "0".to_string()
                    } else {
                        p.comments.get().len().to_string()
                    }
                })
                .needs(["comments"]),
            ))
            .paginate(50)
    }
    fn form(_cx: &Cx) -> Schema {
        Schema::new(TextInput::r#for(Post::fields().title()).required())
    }
}

async fn seed_50(db: &mut Db, tenant: uuid::Uuid) {
    let mut author_ids = Vec::new();
    for i in 0..5 {
        let a = toasty::create!(Author {
            tenant_id: tenant,
            name: format!("Author {i}"),
            email: format!("author{i}@example.com"),
        })
        .exec(db)
        .await
        .expect("create author");
        author_ids.push(a.id);
    }
    // Create 50 posts, each with a comment
    for i in 0..50 {
        let aid = author_ids[i % author_ids.len()];
        let post = toasty::create!(Post {
            tenant_id: tenant,
            title: format!("Post {i:02}"),
            body: format!("Body {i}"),
            status: if i % 2 == 0 {
                "published".to_string()
            } else {
                "draft".to_string()
            },
            featured: i % 3 == 0,
            created_at: Timestamp::now(),
            image_path: "/images/x.jpg".to_string(),
            tags: "bench".to_string(),
            author_id: aid,
        })
        .exec(db)
        .await
        .expect("create post");
        toasty::create!(Comment {
            body: format!("Comment for {i}"),
            post_id: post.id,
        })
        .exec(db)
        .await
        .expect("create comment");
    }
}

/// Fresh request `Cx` for one bench iteration: the pooled `Db` on the app
/// context, the run's tenant (the production seam — the auth layer injects
/// `Tenant`, the bench sets it directly), and real request `Parts` carrying
/// the list URI so `TableState::from_cx` parses a genuine (empty: first page,
/// no search/filter/sort) query instead of the no-request-context early return.
fn bench_cx(db: &Db, tenant: uuid::Uuid) -> Cx {
    let parts = http::Request::builder()
        .uri("/admin/posts")
        .body(())
        .expect("bench request parts")
        .into_parts()
        .0;
    CxTestBuilder::new()
        .app_context(db.clone())
        .request_context(parts)
        .request_context(Tenant(tenant))
        .build()
}

fn summarize(mut times: Vec<f64>) -> (f64, f64, f64, f64, f64) {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = times.len();
    (
        times[n / 2],
        times[(n as f64 * 0.9) as usize % n],
        times[(n as f64 * 0.99) as usize % n],
        times[0],
        times[n - 1],
    )
}

/// The honest list path (GH #171): `TableState::from_cx` → `Table::load`
/// (the tenant-scoped query + the declared `.paginate(50)`, tenancy set, policy
/// enforced) → `render_with_state` → HTML. Fresh `Cx` per iteration.
///
/// This is the exact body of the shipped `panel::load_table_page`
/// (`table.load(cx, scoped_query::<R>(cx)?, state)` behind its paginate guard —
/// `load_table_page` itself is `pub(crate)`, so the detached harness mirrors
/// it rather than calling through). The declared page size is asserted so the
/// `.paginate(50)` on the resource table is genuinely exercised through the
/// loader, not merely declared.
async fn bench_list_path(db: &Db, tenant: uuid::Uuid, iterations: usize) -> Vec<f64> {
    let mut times_ms = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let cx = bench_cx(db, tenant);
        // Policy gate, mirroring `panel::resource_list`: the shipped page
        // 403s without these — a bench that skipped them would not be the
        // list path.
        argentum_core::tenancy::require_tenant(&cx).expect("bench Cx must carry a tenant");
        assert!(
            PostResource::can_view_any(&cx),
            "bench resource must pass can_view_any"
        );
        let start = Instant::now();
        let state = TableState::from_cx(&cx);
        // Action wiring, mirroring `wire_table_actions` (panel/list.rs,
        // `live = false`): the shipped page renders row Delete + bulk bar +
        // Edit chrome per row — a bench without it would under-measure render
        // cost. `wire_table_actions` itself is `pub(crate)`, so the detached
        // harness repeats its steps against the same list URL.
        let table = PostResource::table(&cx);
        assert_eq!(
            table.page_size(),
            Some(50),
            "declared .paginate(50) must reach the loader"
        );
        let table = if PostResource::deletable() {
            table
                .with_delete("/admin/posts".to_string())
                .with_bulk_delete(true)
        } else {
            table
        };
        let table = if PostResource::editable() {
            table.with_edit("/admin/posts".to_string())
        } else {
            table
        };
        let page = table
            .load(
                &cx,
                argentum_core::scoped_query::<PostResource>(&cx).expect("tenant scope"),
                &state,
            )
            .await
            .expect("table load");
        assert_eq!(page.rows.len(), 50, "expected first page of 50 rows");
        let html = table
            .render_with_state(&cx, page, &state, "/admin/posts")
            .await
            .expect("table render")
            .single()
            .await
            .expect("resolve view")
            .render(&cx);
        assert!(
            html.contains("Post 00") && html.contains("Post 49"),
            "rendered HTML must carry the 50 rows"
        );
        times_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times_ms
}

/// Query-only diagnostic (GH #171): tenant-scoped query exec + touching
/// includes on a fresh `Cx` — no `Table::load`, no render. Kept as a labeled
/// diagnostic next to the list-path number; it is not the budget path and is
/// not gated.
async fn bench_query_only(db: &Db, tenant: uuid::Uuid, iterations: usize) -> Vec<f64> {
    let mut times_ms = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let cx = bench_cx(db, tenant);
        let start = Instant::now();
        let mut db2 = argentum_core::db::db(&cx);
        let rows: Vec<Post> = argentum_core::scoped_query::<PostResource>(&cx)
            .expect("tenant scope")
            .exec(&mut db2)
            .await
            .expect("query");
        assert_eq!(rows.len(), 50, "expected 50 rows");
        // Touch includes to ensure they were loaded (no N+1)
        for p in &rows {
            let _ = p.author.get().name.clone();
            let _ = p.comments.get().len();
        }
        times_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times_ms
}

async fn run_leg(db: Db, label: &str, iterations: usize) {
    // Fresh tenant per run: reruns (notably Postgres, a persistent server)
    // stay exact without cleanup — prior runs' rows belong to dead tenants
    // the tenancy-scoped query filters out.
    let tenant = uuid::Uuid::new_v4();
    let mut seed_db = db.clone();
    seed_50(&mut seed_db, tenant).await;
    let (p50, p90, p99, min, max) = summarize(bench_list_path(&db, tenant, iterations).await);
    let (q50, _, _, _, _) = summarize(bench_query_only(&db, tenant, 20).await);
    println!("--- {label} ---");
    println!("tenant: {tenant} (fresh per run; 5 authors + 50 posts + 50 comments)");
    println!(
        "list path (from_cx -> load -> render_with_state -> HTML), {iterations} iters — p50: {p50:.2}ms p90: {p90:.2}ms p99: {p99:.2}ms min: {min:.2}ms max: {max:.2}ms"
    );
    println!("query-only diagnostic (cold Cx, no load/render), 20 iters — p50: {q50:.2}ms");
}

/// Refuse a Postgres URL whose database does not look disposable (GH #171
/// review): the leg drops the named database, so a production URL pasted
/// into the env var must fail loud, never wipe.
fn assert_bench_database(url: &str) {
    let dbname = url
        .rsplit_once('/')
        .map(|(_, tail)| tail.split('?').next().unwrap_or(tail))
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(
        dbname.contains("bench") || dbname.contains("test"),
        "refusing to reset Postgres database {dbname:?}: name a disposable bench/test database"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn bench_database_guard_accepts_bench_names_and_refuses_the_rest() {
        for ok in [
            "postgresql://localhost/argentum_bench",
            "postgresql://u:p@host:5432/my-test-db?sslmode=require",
            "postgresql://localhost/BENCH",
        ] {
            super::assert_bench_database(ok);
        }
        for bad in [
            "postgresql://localhost/production",
            "postgresql://localhost/app",
            "postgresql://localhost/",
            "not-a-url",
        ] {
            let refused = std::panic::catch_unwind(|| super::assert_bench_database(bad));
            assert!(refused.is_err(), "{bad:?} must be refused");
        }
    }
}

async fn run_bench(iterations: usize) {
    println!("=== Argentum Phase-2 bench (GH #171, UNGATED): real list path ===");
    println!("workload: 50 rows, 2 includes (author + comments), tenancy set, policy enforced");
    println!(
        "path: TableState::from_cx -> Table::load (scoped_query + .paginate(50)) -> render_with_state -> HTML"
    );
    println!(
        "budget: <40ms p50 (Phase 2, 50 rows, 2 includes) — reference only; UNGATED while GH #171 collects numbers, gating follows in a follow-up"
    );

    // SQLite leg — always runs.
    let sqlite = Db::builder()
        .models(toasty::models!(Author, Post, Comment))
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    sqlite.push_schema().await.expect("push_schema sqlite");
    run_leg(sqlite, "sqlite (sqlite::memory:)", iterations).await;

    // Postgres leg — runs when pointed at a server (GH #171: SQLite AND
    // Postgres). No local Postgres is assumed: set
    // ARGENTUM_BENCH_POSTGRES_URL to opt in (CI provides the service). The
    // URL must name a disposable bench database — the leg resets it, pushes
    // schema, and seeds under a fresh tenant each run.
    match std::env::var("ARGENTUM_BENCH_POSTGRES_URL") {
        Err(_) => println!(
            "--- postgres --- skipped (set ARGENTUM_BENCH_POSTGRES_URL=postgresql://... to run)"
        ),
        Ok(url) => {
            // The leg drops the named database: refuse anything that does
            // not look disposable, so a pasted production URL fails loud
            // instead of wiping real data.
            assert_bench_database(&url);
            // The bench database is disposable by contract: drop it first so
            // reruns start empty — `push_schema` is not idempotent on
            // Postgres (`relation "authors" already exists` on the second
            // run). Resetting drops the database out from under the handle's
            // pool, so reconnect afterwards. Seeding then uses a fresh tenant
            // per run regardless.
            let pg = Db::builder()
                .models(toasty::models!(Author, Post, Comment))
                .connect(&url)
                .await
                .expect("connect postgres");
            pg.reset_db().await.expect("reset_db postgres");
            drop(pg);
            let pg = Db::builder()
                .models(toasty::models!(Author, Post, Comment))
                .connect(&url)
                .await
                .expect("reconnect postgres");
            pg.push_schema().await.expect("push_schema postgres");
            run_leg(pg, "postgres (ARGENTUM_BENCH_POSTGRES_URL)", iterations).await;
        }
    }
    println!("done (ungated — no PASS/FAIL; the p50 gate follows in a follow-up per GH #171)");
}

#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    Panel::layout_shell(cx, slot).await
}

/// The tenant the HTTP mode seeds and serves. The HTTP mode runs with
/// `Auth::disabled()`, so no auth gate injects a `Tenant`; the layer below
/// supplies this value for every `/admin` request.
const SERVER_TENANT: uuid::Uuid = uuid::Uuid::nil();

/// Supplies the HTTP mode's tenant.
///
/// `enforce_tenant` refuses a tenant-scoped resource with 403 when the request
/// carries no `Tenant` scoped value, so the HTTP mode needs this layer to serve
/// `/admin`. The bench path scopes its own requests instead, through `bench_cx`.
#[layer("/admin")]
async fn inject_server_tenant(cx: &Cx, body: Body, next: Next<'_>) -> Result<Response> {
    next.run(&cx.with(Tenant(SERVER_TENANT)), body).await
}

fn router(db: Db) -> Router {
    Panel::new("admin")
        .app_context(db)
        .auth(argentum_core::Auth::disabled())
        .resource::<AuthorResource>()
        .resource::<PostResource>()
        .build()
        .expect("panel builds")
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_bench = args.iter().any(|a| a == "--bench");
    if is_bench {
        let iterations = args
            .iter()
            .position(|a| a == "--iterations")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100);
        run_bench(iterations).await;
        return;
    }

    // Server mode — build DB, seed, start Topcoat
    let mut db = Db::builder()
        .models(toasty::models!(Author, Post, Comment))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed_50(&mut db, SERVER_TENANT).await;
    let router = router(db);
    println!("storefront-argentum listening on http://localhost:3000/ (try /admin/posts)");
    topcoat::start(router).await.unwrap();
}
