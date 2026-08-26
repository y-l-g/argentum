use argentum_core::{Resource, db::db};
use topcoat::{Result, context::Cx, router::page, view::view};

use crate::models::User;

// Local demo resources — minimal code the snippet shows.
#[derive(Resource)]
#[resource(model = crate::models::User)]
struct BareUserResource;

fn only_ada(_cx: &Cx) -> <User as toasty::schema::Model>::Query<toasty::stmt::List<User>> {
    User::filter(User::fields().name().eq("Ada Lovelace"))
}

#[derive(Resource)]
#[resource(model = crate::models::User, query = only_ada)]
struct ScopedUserResource;

#[page("/admin/showcase/resource")]
async fn resource_showcase(cx: &Cx) -> Result {
    let mut conn = db(cx);

    let all = BareUserResource::query(cx).exec(&mut conn).await?;
    let scoped = ScopedUserResource::query(cx).exec(&mut conn).await?;

    let nav = BareUserResource::navigation();
    let label = nav.label.clone();
    let url = nav.url.clone();

    let all_count = all.len();
    let scoped_count = scoped.len();
    let scoped_name = scoped
        .first()
        .map(|u| u.name.clone())
        .unwrap_or_else(|| "-".to_string());

    view! {
        <div class="ac-showcase">
            <h1>"Resource"</h1>
            <p>"Maps one Toasty Model to its admin UI. One Model → one Resource (CONTEXT.md)."</p>

            <section class="ac-showcase-block">
                <h2>"Derive — model only"</h2>
                <p>"Bare resource uses default query (all rows)."</p>
                <pre><code>"#[derive(Resource)]\n#[resource(model = User)]\nstruct BareUserResource;"</code></pre>
                <div class="ac-showcase-result">
                    <p>"Bare query rows: " (all_count.to_string()) " (expected 2)"</p>
                    <ul>
                        for user in &all {
                            <li>(format!("{} — {}", user.name, user.email))</li>
                        }
                    </ul>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Derive — with query override"</h2>
                <p>"Single seam for tenancy/soft-delete (ADR-0002). Every loader starts from Resource::query."</p>
                <pre><code>"fn only_ada(cx: &Cx) -> Query<List<User>> {\n    User::filter(User::fields().name().eq(\"Ada Lovelace\"))\n}\n#[derive(Resource)]\n#[resource(model = User, query = only_ada)]\nstruct ScopedUserResource;"</code></pre>
                <div class="ac-showcase-result">
                    <p>"Scoped query rows: " (scoped_count.to_string()) " (expected 1)"</p>
                    <p>"First name: " (scoped_name)</p>
                    <ul>
                        for user in &scoped {
                            <li>(format!("{} — {}", user.name, user.email))</li>
                        }
                    </ul>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"NavigationItem"</h2>
                <p>"Derived from Model type name (User → Users) and Panel prefix."</p>
                <pre><code>"BareUserResource::navigation() // → NavigationItem { label: \"Users\", url: \"/admin\" }"</code></pre>
                <div class="ac-showcase-result">
                    <p>"label: " (&label) ", url: " (&url)</p>
                    <p><a href=(url.clone()) class="ac-nav-item">(label.clone())</a>" ← nav link"</p>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Variants"</h2>
                <p>"Duplicate model/query → compile error, unknown key → error (argentum-macros/src/lib.rs:22-45)."</p>
                <pre><code>"#[resource(model = User, model = User)] // error: duplicate `model`\n#[resource(model = User, query = only_ada, query = only_ada)] // error: duplicate `query`\n#[resource(model = User, unknown = Foo)] // error: unknown key"</code></pre>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        </div>
    }
}
