use argentum_core::Panel;
use topcoat::{
    Result,
    router::page,
    view::{View, view},
};

#[page("/admin/showcase/panel")]
async fn panel_showcase() -> Result<impl View> {
    let p_admin = Panel::new("admin").prefix().to_string();
    let p_slash = Panel::new("/admin").prefix().to_string();
    let p_slash_trail = Panel::new("admin/").prefix().to_string();
    let p_empty = Panel::new("").prefix().to_string();
    let p_custom = Panel::new("showcase").prefix().to_string();

    Ok(view! {
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Panel")
                argentum_ui::page_description(
                    "Admin shell — owns Router, Db in app_context, and prefix. Single Panel in Phase 1."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Construction"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Panel::new(\"admin\")\n    .app_context(db)\n    .resource::<UserResource>()\n    .build() // → Router via discover + app_context\n// mounts UserResource at \"/admin/users\" and redirects \"/admin\""
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">
                        "Current app mounts at: "
                        (p_admin.clone())
                    </p>
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Prefix normalization — variants"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Prefixes are trimmed and normalized to /{name}. Empty → /admin."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Panel::new(\"admin\").prefix()    // \"/admin\"\nPanel::new(\"/admin\").prefix()   // \"/admin\"\nPanel::new(\"admin/\").prefix()   // \"/admin\"\nPanel::new(\"\").prefix()         // \"/admin\"\nPanel::new(\"showcase\").prefix() // \"/showcase\""
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <table class="w-full caption-bottom border-collapse text-sm">
                        <thead>
                            <tr>
                                <th>"input"</th>
                                <th>"prefix()"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr>
                                <td>"\"admin\""</td>
                                <td>(p_admin)</td>
                            </tr>
                            <tr>
                                <td>"\"/admin\""</td>
                                <td>(p_slash)</td>
                            </tr>
                            <tr>
                                <td>"\"admin/\""</td>
                                <td>(p_slash_trail)</td>
                            </tr>
                            <tr>
                                <td>"\"\""</td>
                                <td>(p_empty)</td>
                            </tr>
                            <tr>
                                <td>"\"showcase\""</td>
                                <td>(p_custom)</td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Db in app_context"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "let router = Panel::new(\"admin\").app_context(db).build();\n// later in page/shard: let mut db = db(cx); // app_context::<Db>(cx).clone()\n// Db is Arc-pooled, clone is cheap, exec needs &mut Db"
                )
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        )
    })
}
