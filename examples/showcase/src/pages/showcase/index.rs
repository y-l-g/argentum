use topcoat::{
    Result,
    router::page,
    view::{View, view},
};

/// The showcased pages: link target, label, and what each one demonstrates.
const FEATURES: &[(&str, &str, &str)] = &[
    (
        "/admin/showcase/ui",
        "UI",
        "argentum-ui seam: every component family, with Tokens (proves the Tailwind seam)",
    ),
    (
        "/admin/showcase/dialog",
        "Dialog",
        "shadcn/Sonner toast stack (fixed bottom-right) + alert_dialog with Destructive/Outline buttons",
    ),
    (
        "/admin/showcase/panel",
        "Panel",
        "app shell, prefix, Db in app_context, Router discover",
    ),
    (
        "/admin/showcase/resource",
        "Resource",
        "#[derive(Resource)] model + query override, navigation",
    ),
    (
        "/admin/showcase/schema",
        "Schema",
        "Section / Group / Grid / TextInput + composition variants",
    ),
    (
        "/admin/showcase/table",
        "Table",
        "TextColumn searchable/sortable, live page-owned signals + a shard; /admin/users is the Resource seam",
    ),
    (
        "/admin/showcase/db",
        "Db + memoize",
        "db(cx) glue and per-request memoization",
    ),
    (
        "/admin/users",
        "Admin list (/admin/users)",
        "real app page (Panel + Resource → table)",
    ),
];

#[page("/admin/showcase")]
async fn showcase_index() -> Result<impl View> {
    Ok(view! {
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Showcase")
                argentum_ui::page_description(
                    "Single example demonstrating every Argentum feature — minimal snippet, description, and live result."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Features"
                </h2>
                argentum_ui::table(
                    argentum_ui::table_header(
                        argentum_ui::table_row(
                            argentum_ui::table_head("Page")
                            argentum_ui::table_head("What it shows")
                        )
                    )
                    argentum_ui::table_body(
                        for (href, label, description) in FEATURES {
                            argentum_ui::table_row(
                                argentum_ui::table_cell(
                                    <a
                                        href=(*href)
                                        class="font-medium text-primary hover:underline"
                                    >
                                        (*label)
                                    </a>
                                )
                                argentum_ui::table_cell(
                                    // `table_cell` is `whitespace-nowrap`;
                                    // descriptions need to wrap or the table
                                    // scrolls sideways (GH #151 §4).
                                    <span class="whitespace-normal">(*description)</span>
                                )
                            )
                        }
                    )
                )
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "How to read each page"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Each page shows: description → minimal code snippet → rendered result as in a real app."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "// snippet (copy-paste minimal)\n// rendered below is the live View from that code"
                )
            </section>
        )
    })
}
