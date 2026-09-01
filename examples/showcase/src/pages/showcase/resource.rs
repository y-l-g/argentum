use argentum_core::{Resource, db::db};
use topcoat::{
    Result,
    context::Cx,
    router::page,
    view::{View, view},
};

use crate::models::User;

// Local demo resources — minimal code the snippet shows.
#[derive(Resource)]
#[resource(model = crate::models::User)]
struct BareUserResource;

fn only_ada(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
    toasty::stmt::Query::<toasty::stmt::List<User>>::all()
        .filter(User::fields().name().eq("Ada Lovelace"))
}

#[derive(Resource)]
#[resource(model = crate::models::User, query = only_ada)]
struct ScopedUserResource;

#[page("/admin/showcase/resource")]
async fn resource_showcase(cx: &Cx) -> Result<impl View> {
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

    Ok(view! {
            argentum_ui::page(
                argentum_ui::page_header(
                    argentum_ui::page_title("Resource")
                    argentum_ui::page_description(
                        "Maps one Toasty Model to its admin UI. One Model → one Resource (CONTEXT.md)."
                    )
                )

                <section
                    class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
                >
                    <h2 class="text-lg font-semibold tracking-tight text-foreground">
                        "Derive — model only"
                    </h2>
                    <p class="text-sm text-muted-foreground">
                        "Bare resource uses default query (all rows)."
                    </p>
                    argentum_ui::code_block(
                        lang: "rust",
                        code: "#[derive(Resource)]\n#[resource(model = User)]\nstruct BareUserResource;"
                    )
                    <div class="rounded-lg border border-border bg-background p-4">
                        <p class="text-sm text-muted-foreground">
                            "Bare query rows: "
                            (all_count.to_string())
                            " (expected 3)"
                        </p>
                        <ul>
                            for user in &all {
                                <li>(format!("{} — {}", user.name, user.email))</li>
                            }
                        </ul>
                    </div>
                </section>

                <section
                    class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
                >
                    <h2 class="text-lg font-semibold tracking-tight text-foreground">
                        "Derive — with query override"
                    </h2>
                    <p class="text-sm text-muted-foreground">
                        "Single seam for tenancy/soft-delete (ADR-0002). Every loader starts from Resource::query."
                    </p>
                    argentum_ui::code_block(
                        lang: "rust",
                        code: "fn only_ada(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {\n    toasty::stmt::Query::<toasty::stmt::List<User>>::all()\n        .filter(User::fields().name().eq(\"Ada Lovelace\"))\n}\n#[derive(Resource)]\n#[resource(model = User, query = only_ada)]\nstruct ScopedUserResource;"
                    )
                    <div class="rounded-lg border border-border bg-background p-4">
                        <p class="text-sm text-muted-foreground">
                            "Scoped query rows: "
                            (scoped_count.to_string())
                            " (expected 1)"
                        </p>
                        <p class="text-sm text-muted-foreground">
                            "First name: "
                            (scoped_name)
                        </p>
                        <ul>
                            for user in &scoped {
                                <li>(format!("{} — {}", user.name, user.email))</li>
                            }
                        </ul>
                    </div>
                </section>

                <section
                    class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
                >
                    <h2 class="text-lg font-semibold tracking-tight text-foreground">
                        "NavigationItem"
                    </h2>
                    <p class="text-sm text-muted-foreground">
                        "Derived from Model type name (User → Users) and Panel prefix."
                    </p>
                    argentum_ui::code_block(
                        lang: "rust",
                        code: "BareUserResource::navigation() // → NavigationItem { label: \"Users\", url: \"/admin/bare-users\" }"
                    )
                    <div class="rounded-lg border border-border bg-background p-4">
                        <p class="text-sm text-muted-foreground">
                            "label: "
                            (&label)
                            ", url: "
                            (&url)
                        </p>
                        <p>
                            <a
                                href=(url.clone())
                                class="text-sm text-primary hover:underline"
                            >
                                (label.clone())
                            </a>
                            " ← nav link"
                        </p>
                    </div>
                </section>

                <section
                    class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
                >
                    <h2 class="text-lg font-semibold tracking-tight text-foreground">
                        "Variants"
                    </h2>
                    <p class="text-sm text-muted-foreground">
                        "Duplicate model/query → compile error, unknown key → error (argentum-macros/src/lib.rs:22-45)."
                    </p>
                    argentum_ui::code_block(
                        lang: "rust",
                        code: "#[resource(model = User, model = User)] // error: duplicate `model`\n#[resource(model = User, query = only_ada, query = only_ada)] // error: duplicate `query`\n#[resource(model = User, unknown = Foo)] // error: unknown key"
                    )
                </section>

                <p><a href="/admin/showcase">"← back to showcase"</a></p>
            )
    })
}
