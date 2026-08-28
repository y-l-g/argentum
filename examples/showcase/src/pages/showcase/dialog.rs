use topcoat::{Result, context::Cx, router::page, view::view};

#[page("/admin/showcase/dialog")]
async fn dialog_showcase(cx: &Cx) -> Result {
    // Prove Notification and Dialog chrome — all Token-only, no ac-*
    // Notification: card with border-border bg-background shadow-sm, fixed stack
    // Dialog: alert_dialog with card_header/card_footer and Primary/Destructive buttons
    let dialog = view! {
        cx =>
        argentum_ui::alert_dialog(
            open: true,
            argentum_ui::dialog_content(
                argentum_ui::dialog_header(
                    argentum_ui::dialog_title("Delete user?")
                    argentum_ui::dialog_description("This action cannot be undone. The user will be permanently deleted.")
                )
                argentum_ui::dialog_footer(
                    argentum_ui::button(variant: argentum_ui::ButtonVariant::Outline, "Cancel")
                    argentum_ui::button(variant: argentum_ui::ButtonVariant::Destructive, "Delete")
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
                argentum_ui::page_description("Notifications render in a top-level Boundary owned by the Panel Shell (fixed top-4 right-4), each a card with border-border bg-background shadow-sm. Dialogs use alert_dialog with card_header/card_footer and Primary/Destructive buttons.")
            )

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2 class="text-lg font-semibold text-foreground">"Notification (card in fixed stack)"</h2>
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
                <h2 class="text-lg font-semibold text-foreground">"Dialog / AlertDialog"</h2>
                argentum_ui::code_block(lang: "rust", code: "alert_dialog(open: true,\n    dialog_content(\n        dialog_header(dialog_title(\"Delete user?\"))\n        dialog_footer(button(Outline, \"Cancel\") button(Destructive, \"Delete\"))\n    )\n)")
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">"Destructive Actions that requires_confirmation() open alert_dialog with card_header/card_footer and Primary/Destructive variants."</p>
                    // The dialog is open:true, so it will render as a <dialog open> with backdrop and card
                    (dialog)
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2 class="text-lg font-semibold text-foreground">"Token-only customization"</h2>
                <p class="text-sm text-muted-foreground">"Edit Tokens in styles.css :root/.dark (--background, --foreground, --primary, --border, --ring, etc.) to re-theme the whole diceboard. Additive class is allowed only on Panel::shell and Section/card containers (narrow seam, no per-cell attrs in v1)."</p>
                argentum_ui::code_block(lang: "rust", code: "Section::new(\"Account\").class(\"max-w-2xl\").schema(...)\nPanel::render_shell(cx, nav, current, slot, Some(\"bg-muted\"))\n/* :root { --primary: oklch(...); } .dark { --primary: ...; } */")
            </section>

            <p><a href="/admin/showcase" class="text-sm text-primary hover:underline">"← back to showcase"</a></p>
        )
    }
}
