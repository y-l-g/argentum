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
                    argentum_ui::table(
                        argentum_ui::table_header(
                            argentum_ui::table_row(
                                argentum_ui::table_head("input")
                                argentum_ui::table_head("prefix()")
                            )
                        )
                        argentum_ui::table_body(
                            argentum_ui::table_row(
                                argentum_ui::table_cell("\"admin\"")
                                argentum_ui::table_cell((p_admin))
                            )
                            argentum_ui::table_row(
                                argentum_ui::table_cell("\"/admin\"")
                                argentum_ui::table_cell((p_slash))
                            )
                            argentum_ui::table_row(
                                argentum_ui::table_cell("\"admin/\"")
                                argentum_ui::table_cell((p_slash_trail))
                            )
                            argentum_ui::table_row(
                                argentum_ui::table_cell("\"\"")
                                argentum_ui::table_cell((p_empty))
                            )
                            argentum_ui::table_row(
                                argentum_ui::table_cell("\"showcase\"")
                                argentum_ui::table_cell((p_custom))
                            )
                        )
                    )
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
