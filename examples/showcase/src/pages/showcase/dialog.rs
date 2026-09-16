use std::collections::HashMap;

use argentum_core::{Notification, csrf, notification::set_notification};
use topcoat::{
    Result,
    context::Cx,
    router::{content::Form, error::see_other, page, query_params},
    view::{View, attributes, view},
};

use super::example::example;

#[query_params]
struct DialogQuery {
    open: Option<bool>,
}

/// Flash one Notification and land back on the dialog page (POST/Redirect/Get).
///
/// `status` picks the variant; the shell's toaster renders it on the next GET.
/// `Result<()>` is the documented shape for a page that always redirects:
/// the redirect rides `Err`, so there is no `Ok` view to infer.
#[page(POST "/admin/showcase/dialog/notify")]
async fn dialog_notify(cx: &Cx, Form(values): Form<HashMap<String, String>>) -> Result<()> {
    csrf::verify(cx, &values)?;
    let notification = match values.get("status").map(String::as_str) {
        Some("success") => Notification::success("User created")
            .description("Ada Lovelace was added successfully."),
        Some("warning") => {
            Notification::warning("Careful").description("This action changes stored data.")
        }
        Some("error") => {
            Notification::error("Something failed").description("Nothing was changed.")
        }
        _ => Notification::info("Heads up").description("The record already exists."),
    };
    set_notification(cx, notification);
    Err(see_other("/admin/showcase/dialog").into())
}

#[page("/admin/showcase/dialog")]
async fn dialog_showcase(cx: &Cx) -> Result<impl View> {
    // Prove Notification and Dialog chrome — all Token-only, no ac-*
    // Notification: the shadcn/Sonner toast surface in a fixed bottom-right
    // stack, flashed by the POST above and rendered by the shell.
    // Dialog: alert_dialog driven by ?open= — Cancel/Delete are links back to
    // the plain page (SSR close), dialog.js adds Escape/backdrop dismissal
    // without reload.
    let open = query_params::<DialogQuery>(cx)
        .ok()
        .and_then(|q| q.open)
        .unwrap_or(false);
    // `ensure_token` sets the CSRF cookie when the form is (re)rendered, so
    // `dialog_notify` can verify it — the same contract as every real form.
    let csrf_token = csrf::ensure_token(cx);
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

            example(
                title: "Toast (shadcn/Sonner)",
                description: "A handler flashes a Notification; the shell's toaster mounts it, shows it bottom-right, and notifications.js auto-dismisses it after 4s (paused on hover or focus). The stack lives outside the streamed region, so a grid swap cannot drop it.",
                code: "// POST /admin/showcase/dialog/notify\nset_notification(\n    cx,\n    Notification::success(\"User created\").description(\"Ada Lovelace was added successfully.\"),\n);\nErr(see_other(\"/admin/showcase/dialog\").into()) // PRG",
                <p class="text-sm text-muted-foreground">
                    "Trigger a real toast — it lands in the shell's toaster, bottom-right."
                </p>
                <div class="flex flex-wrap items-center gap-2">
                    for (status, label) in [
                        ("success", "Success"),
                        ("info", "Info"),
                        ("warning", "Warning"),
                        ("error", "Error"),
                    ] {
                        <form method="post" action="/admin/showcase/dialog/notify">
                            <input
                                type="hidden"
                                name=(csrf::FIELD_NAME)
                                value=(csrf_token.clone())
                            >
                            <input type="hidden" name="status" value=(status)>
                            argentum_ui::button(
                                variant: argentum_ui::ButtonVariant::Outline,
                                attrs: attributes! { type="submit" },
                                (label)
                            )
                        </form>
                    }
                </div>
            )

            example(
                title: "Dialog / AlertDialog",
                description: "Destructive actions that require confirmation open alert_dialog with Outline/Destructive answers. Cancel and Delete are links back to the plain page; with dialog.js, Escape, the backdrop, and the same links close without a reload.",
                code: "// GET /admin/showcase/dialog?open=true\nlet open = query_params::<DialogQuery>(cx).ok().and_then(|q| q.open).unwrap_or(false);\nalert_dialog(\n    open: open,\n    dialog_content(\n        dialog_header(dialog_title(\"Delete user?\"))\n        dialog_footer(\n            <a href=\"/admin/showcase/dialog\" data-dialog-close class=(button_variants(Outline, Md))>\"Cancel\"</a>\n            <a href=\"/admin/showcase/dialog\" data-dialog-close class=(button_variants(Destructive, Md))>\"Delete\"</a>\n        )\n    )\n)",
                <a
                    href="/admin/showcase/dialog?open=true"
                    class=(argentum_ui::button_variants(
                        argentum_ui::ButtonVariant::Primary,
                        argentum_ui::ButtonSize::Md,
                    ))
                >
                    "Open dialog"
                </a>
                // The dialog is open only when the query says so, so the page
                // is readable and exitable; the state survives a reload and a
                // link.
                (dialog)
            )

            <p>
                <a href="/admin/showcase" class="text-sm text-primary hover:underline">
                    "← back to showcase"
                </a>
            </p>
        )
    })
}
