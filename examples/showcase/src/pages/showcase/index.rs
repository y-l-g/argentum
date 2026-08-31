use topcoat::{Result, router::page, view::view};

#[page("/admin/showcase")]
async fn showcase_index() -> Result {
    view! {
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
                <ul>
                    <li>
                        <a href="/admin/showcase/ui">"UI"</a>
                        " — argentum-ui seam: card, button, badge with Tokens (proves Tailwind seam)"
                    </li>
                    <li>
                        <a href="/admin/showcase/dialog">"Dialog"</a>
                        " — notification stack (fixed top-4 right-4, card) + alert_dialog with Primary/Destructive buttons"
                    </li>
                    <li>
                        <a href="/admin/showcase/panel">"Panel"</a>
                        " — app shell, prefix, Db in app_context, Router discover"
                    </li>
                    <li>
                        <a href="/admin/showcase/resource">"Resource"</a>
                        " — #[derive(Resource)] model + query override, navigation"
                    </li>
                    <li>
                        <a href="/admin/showcase/schema">"Schema"</a>
                        " — Section / Group / Grid / TextInput + composition variants"
                    </li>
                    <li>
                        <a href="/admin/showcase/table">"Table"</a>
                        " — TextColumn searchable/sortable, Table::for + columns"
                    </li>
                    <li>
                        <a href="/admin/showcase/db">"Db + memoize"</a>
                        " — db(cx) glue and per-request memoization"
                    </li>
                    <li>
                        <a href="/admin/users">"Admin list (/admin/users)"</a>
                        " — real app page (Panel + Resource → table)"
                    </li>
                </ul>
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
    }
}
