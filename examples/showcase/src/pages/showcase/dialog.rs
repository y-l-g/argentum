use argentum_core::{Notification, notification::live_toast};
use topcoat::{
    Result,
    context::Cx,
    router::page,
    runtime::{Event, procedure, signal},
    view::{View, attributes, view},
};

use super::example::example;

/// The server side of the toast transport (GH #154 §3): builds the real
/// [`Notification`] and returns its fields.
///
/// A procedure's return must belong to the shared vocabulary, so the toast
/// crosses as `(status, title, description)`; the click handler writes them
/// into the page's [`live_toast`] signals and the shell's live toaster mounts
/// the surface in place — no POST/Redirect/Get, no scroll reset.
#[procedure]
pub async fn showcase_notify(status: String) -> Result<(String, String, String)> {
    let notification = match status.as_str() {
        "success" => Notification::success("User created")
            .description("Ada Lovelace was added successfully."),
        "warning" => {
            Notification::warning("Careful").description("This action changes stored data.")
        }
        "error" => Notification::error("Something failed").description("Nothing was changed."),
        _ => Notification::info("Heads up").description("The record already exists."),
    };
    Ok((
        notification.status.as_str().to_string(),
        notification.title,
        notification.description.unwrap_or_default(),
    ))
}

#[page("/admin/showcase/dialog")]
async fn dialog_showcase(cx: &Cx) -> Result<impl View> {
    // All in-place (GH #154 §3): the toast buttons call `showcase_notify` and
    // write its result into the page's live-toast signals (the shell's
    // `live_toaster` shard mounts the real Sonner surface); the dialog's open
    // state is a signal, so the trigger opens it without a navigation and
    // Cancel/Delete/Escape/backdrop just flip it back.
    let toast = live_toast(cx);
    let toast_status = toast.status.clone();
    let toast_title = toast.title.clone();
    let toast_description = toast.description.clone();
    let toast_serial = toast.serial.clone();
    let dialog_open = signal(cx, || false);
    // The dialog's `open` binding captures its own handle; the trigger below
    // keeps the original.
    let dialog_open_view = dialog_open.clone();
    // `dialog.js` closes the `<dialog>` itself on Escape/backdrop; the
    // element's `@close` event keeps the signal in step, so the trigger can
    // reopen it afterwards.
    let close_signal = dialog_open.clone();
    let dialog = view! {
        cx =>
        argentum_ui::alert_dialog(
            open: $(dialog_open_view.get()),
            attrs: attributes! {
                aria-labelledby="showcase-dialog-title"
                aria-describedby="showcase-dialog-description"
                @close=$(|_e: Event| close_signal.set(false))
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
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Outline,
                        attrs: attributes! { type="button" data-dialog-close="" },
                        "Cancel"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Destructive,
                        attrs: attributes! { type="button" data-dialog-close="" },
                        "Delete"
                    )
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
                    "Notifications render as shadcn/Sonner toasts in the shell's stack (fixed bottom-right). Dialogs use alert_dialog driven by a signal — open it from the button below; Cancel/Delete/Escape/backdrop close it. Both interactions stay on the page: no reload, no scroll jump."
                )
            )

            example(
                title: "Toast (shadcn/Sonner)",
                description: "Each button calls a #[procedure] that builds the real Notification server-side; the handler writes the result into the page's live-toast signals and the shell's toaster mounts the surface in place, bottom-right. notifications.js auto-dismisses it after 4s (paused on hover or focus). No POST, no redirect, no reload.",
                code: "#[procedure] showcase_notify(status) -> (status, title, description)\nlet n = showcase_notify(status).await;\ntoast.status.set(n.0); // the shell's live_toaster shard re-renders\ntoast.title.set(n.1); // the real Sonner toast, in place\ntoast.serial.increment();",
                <p class="text-sm text-muted-foreground">
                    "Trigger a real toast — it mounts in the shell's toaster, bottom-right, without leaving the page."
                </p>
                <div class="flex flex-wrap items-center gap-2">
                    for (status, label) in [
                        ("success", "Success"),
                        ("info", "Info"),
                        ("warning", "Warning"),
                        ("error", "Error"),
                    ] {
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Outline,
                            attrs: attributes! {
                                type="button"
                                @click=$(async |_e: Event| {
                                    let n = showcase_notify(status.to_owned()).await;
                                    toast_status.set(n.0);
                                    toast_title.set(n.1);
                                    toast_description.set(n.2);
                                    toast_serial.increment();
                                })
                            },
                            (label)
                        )
                    }
                </div>
            )

            example(
                title: "Dialog / AlertDialog",
                description: "Destructive actions that require confirmation open alert_dialog from a signal. Cancel and Delete close it through dialog.js; Escape and the backdrop dismiss it the same way, and the dialog's @close handler keeps the signal in step so it can be reopened.",
                code: "let open = signal(cx, || false);\n// Open\nbutton(variant: Primary, attrs: attributes! { @click=$(|_e| open.set(true)) }, \"Open dialog\")\n// The dialog binds the signal; Cancel/Delete dismiss through dialog.js\nalert_dialog(open: $(open.get()), attrs: attributes! { @close=$(|_e| open.set(false)) }, ...)",
                argentum_ui::button(
                    variant: argentum_ui::ButtonVariant::Primary,
                    attrs: attributes! {
                        type="button"
                        @click=$(|_e: Event| dialog_open.set(true))
                    },
                    "Open dialog"
                )
                // The dialog is closed on first paint and opens only from the
                // signal; `dialog.js` adds Escape/backdrop dismissal.
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
