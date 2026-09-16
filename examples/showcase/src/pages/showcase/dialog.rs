use topcoat::{
    Result,
    context::Cx,
    icon::icon,
    router::{page, query_params},
    view::{View, attributes, view},
};

#[query_params]
struct DialogQuery {
    open: Option<bool>,
}

#[page("/admin/showcase/dialog")]
async fn dialog_showcase(cx: &Cx) -> Result<impl View> {
    // Prove Notification and Dialog chrome — all Token-only, no ac-*
    // Notification: the shadcn/Sonner toast surface in a fixed bottom-right stack
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
            attrs: attributes! {
                aria-labelledby="showcase-dialog-title"
                aria-describedby="showcase-dialog-description"
            },
            argentum_ui::dialog_content(
                argentum_ui::dialog_header(
                    argentum_ui::dialog_title(
                        attrs: attributes! { id="showcase-dialog-title" },
                        "Delete user?"
                    )
                    argentum_ui::dialog_description(
                        attrs: attributes! { id="showcase-dialog-description" },
                        "This action cannot be undone. The user will be permanently deleted."
                    )
                )
                argentum_ui::dialog_footer(
                    <a
                        href="/admin/showcase/dialog"
                        data-dialog-close=""
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Outline,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Cancel"
                    </a>
                    <a
                        href="/admin/showcase/dialog"
                        data-dialog-close=""
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Destructive,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Delete"
                    </a>
                )
            )
        )
    };
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Dialog & Notification — diceboard polish")
                argentum_ui::page_description(
                    "Notifications render as shadcn/Sonner toasts in a top-level stack owned by the Panel Shell (fixed bottom-right). Dialogs use alert_dialog driven by ?open= — open it from the button below; Cancel/Delete/Escape/backdrop close it."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Toast (shadcn/Sonner)"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "// Panel::render_shell owns the toaster (Sonner surface):\n//   <ol data-sonner-toaster class=\"fixed right-4 bottom-4 ... w-[356px] flex-col gap-3.5\">\n//     <li data-sonner-toast data-type=\"success\">icon + title + description + close</li>\n//   </ol>"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">
                        "Toasts stack bottom-right, auto-dismiss after 4s (paused on hover/focus), and close from the circular button. A mutation flashes the same markup through the shell's toaster and it survives Boundary swaps."
                    </p>
                    // Static visual proof of the toast surface; the shell's
                    // real toast (armed by notifications.js) is the live one.
                    <div
                        class="mt-4 flex max-w-full items-center gap-1.5 rounded-lg border border-border bg-background p-4 text-[13px] text-foreground shadow-lg sm:w-[356px]"
                    >
                        <span class="flex size-4 shrink-0 items-center justify-center">
                            icon(
                                data: argentum_ui::icons::CIRCLE_CHECK,
                                attrs: attributes! { class="size-4" }
                            )
                        </span>
                        <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                            <div class="font-medium leading-normal">"User created"</div>
                            <div class="leading-snug text-muted-foreground">
                                "Ada Lovelace was added successfully."
                            </div>
                        </div>
                    </div>
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Dialog / AlertDialog"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "// GET /admin/showcase/dialog?open=true\nlet open = query_params::<DialogQuery>(cx).ok().and_then(|q| q.open).unwrap_or(false);\nalert_dialog(open: open,\n    dialog_content(\n        dialog_header(dialog_title(\"Delete user?\"))\n        dialog_footer(<a href=\"/admin/showcase/dialog\" data-dialog-close class=(button_variants(Outline, Md))>\"Cancel\"</a> <a href=\"...\" data-dialog-close class=(button_variants(Destructive, Md))>\"Delete\"</a>)\n    )\n)"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">
                        "Destructive actions that require confirmation open alert_dialog with card_header/card_footer and Outline/Destructive answers. Cancel and Delete are links back to the plain page; with dialog.js, Escape, the backdrop, and the same links close without a reload."
                    </p>
                    <a
                        href="/admin/showcase/dialog?open=true"
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Primary,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Open dialog"
                    </a>
                    // The dialog is open only when the query says so, so the page is
                    // readable and exitable; the state survives a reload and a link.
                    (dialog)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Token-only customization"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Edit Tokens in styles.css :root/.dark (--background, --foreground, --primary, --border, --ring, etc.) to re-theme the whole diceboard. Additive class is allowed only on Panel::shell and Section/card containers (narrow seam, no per-cell attrs in v1)."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Section::new(\"Account\").class(\"max-w-2xl\").schema(...)\nPanel::render_shell(cx, nav, current, slot, Some(\"bg-muted\"))\n/* :root { --primary: oklch(...); } .dark { --primary: ...; } */"
                )
            </section>

            <p>
                <a href="/admin/showcase" class="text-sm text-primary hover:underline">
                    "← back to showcase"
                </a>
            </p>
        )
    })
}
