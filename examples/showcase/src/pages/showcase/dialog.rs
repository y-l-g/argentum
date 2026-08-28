use topcoat::{
    Result,
    context::Cx,
    router::{page, query_params},
    view::{attributes, view},
};

#[query_params]
struct DialogQuery {
    open: Option<bool>,
}

#[page("/admin/showcase/dialog")]
async fn dialog_showcase(cx: &Cx) -> Result {
    // Prove Notification and Dialog chrome — all Token-only, no ac-*
    // Notification: card with border-border bg-background shadow-sm, fixed stack
    // Dialog: alert_dialog driven by ?open= — Cancel/Delete are links back to
    // the plain page (SSR close), dialog.js adds Escape/backdrop dismissal
    // without reload.
    let open = query_params::<DialogQuery>(cx)
        .ok()
        .and_then(|q| q.open)
        .unwrap_or(false);
    let dialog = view! {
        cx =>
        argentum_ui::alert_dialog(
            open: open,
            attrs: attributes! { aria-labelledby="showcase-dialog-title" aria-describedby="showcase-dialog-description" },
            argentum_ui::dialog_content(
                argentum_ui::dialog_header(
                    argentum_ui::dialog_title(attrs: attributes! { id="showcase-dialog-title" }, "Delete user?")
                    argentum_ui::dialog_description(attrs: attributes! { id="showcase-dialog-description" }, "This action cannot be undone. The user will be permanently deleted.")
                )
                argentum_ui::dialog_footer(
                    <a href="/admin/showcase/dialog" data-dialog-close="" class=(argentum_ui::button_variants(argentum_ui::ButtonVariant::Outline, argentum_ui::ButtonSize::Md))>"Cancel"</a>
                    <a href="/admin/showcase/dialog" data-dialog-close="" class=(argentum_ui::button_variants(argentum_ui::ButtonVariant::Destructive, argentum_ui::ButtonSize::Md))>"Delete"</a>
                )
            )
        )
    }
    .unwrap();
    view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Dialog & Notification — diceboard polish")
                argentum_ui::page_description("Notifications render in a top-level Boundary owned by the Panel Shell (fixed top-4 right-4), each a card with border-border bg-background shadow-sm. Dialogs use alert_dialog driven by ?open= — open it from the button below; Cancel/Delete/Escape/backdrop close it.")
            )

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2 class="text-lg font-semibold tracking-tight text-foreground">"Notification (card in fixed stack)"</h2>
                argentum_ui::code_block(lang: "rust", code: "// Panel::render_shell owns: <div class=\"fixed top-4 right-4 z-50 flex flex-col gap-2\">\n//   card(border-border bg-background shadow-sm, \"User created\")\n// </div>")
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">"Notification stack is fixed top-right, survives Boundary swaps (Panel Shell top-level Boundary)."</p>
                    // Render a sample notification card inline for visual proof
                    <div class="mt-4 flex flex-col gap-2 rounded-xl border border-border bg-background p-4 shadow-sm">
                        <p class="text-sm font-medium text-foreground">"User created"</p>
                        <p class="text-sm text-muted-foreground">"Ada Lovelace was added successfully."</p>
                    </div>
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2 class="text-lg font-semibold tracking-tight text-foreground">"Dialog / AlertDialog"</h2>
                argentum_ui::code_block(lang: "rust", code: "// GET /admin/showcase/dialog?open=true\nlet open = query_params::<DialogQuery>(cx).ok().and_then(|q| q.open).unwrap_or(false);\nalert_dialog(open: open,\n    dialog_content(\n        dialog_header(dialog_title(\"Delete user?\"))\n        dialog_footer(<a href=\"/admin/showcase/dialog\" data-dialog-close class=(button_variants(Outline, Md))>\"Cancel\"</a> <a href=\"...\" data-dialog-close class=(button_variants(Destructive, Md))>\"Delete\"</a>)\n    )\n)")
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">"Destructive actions that require confirmation open alert_dialog with card_header/card_footer and Outline/Destructive answers. Cancel and Delete are links back to the plain page; with dialog.js, Escape, the backdrop, and the same links close without a reload."</p>
                    <a href="/admin/showcase/dialog?open=true" class=(argentum_ui::button_variants(argentum_ui::ButtonVariant::Primary, argentum_ui::ButtonSize::Md))>"Open dialog"</a>
                    // The dialog is open only when the query says so, so the page is
                    // readable and exitable; the state survives a reload and a link.
                    (dialog)
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2 class="text-lg font-semibold tracking-tight text-foreground">"Token-only customization"</h2>
                <p class="text-sm text-muted-foreground">"Edit Tokens in styles.css :root/.dark (--background, --foreground, --primary, --border, --ring, etc.) to re-theme the whole diceboard. Additive class is allowed only on Panel::shell and Section/card containers (narrow seam, no per-cell attrs in v1)."</p>
                argentum_ui::code_block(lang: "rust", code: "Section::new(\"Account\").class(\"max-w-2xl\").schema(...)\nPanel::render_shell(cx, nav, current, slot, Some(\"bg-muted\"))\n/* :root { --primary: oklch(...); } .dark { --primary: ...; } */")
            </section>

            <p><a href="/admin/showcase" class="text-sm text-primary hover:underline">"← back to showcase"</a></p>
        )
    }
}
