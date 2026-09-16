use topcoat::{
    Result,
    context::Cx,
    router::page,
    view::{View, view},
};

#[page("/admin/showcase/ui")]
async fn ui_showcase(cx: &Cx) -> Result<impl View> {
    // Prove the argentum-ui seam: card + button with Token classes, no ac-*
    let card_view = view! {
        cx =>
        argentum_ui::card(
            argentum_ui::card_header(
                argentum_ui::card_title("Beautiful card")
                argentum_ui::card_description(
                    "Tokens: bg-background, border-border, shadow-sm, text-muted-foreground"
                )
            )
            argentum_ui::card_content(
                <p class="text-sm text-muted-foreground">
                    "This card proves the Tailwind seam via argentum-ui. Tokens on :root/.dark are the single customization seam."
                </p>
                <div class="mt-4 flex gap-2">
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Primary,
                        "Primary"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Outline,
                        "Outline"
                    )
                    argentum_ui::badge(
                        variant: argentum_ui::BadgeVariant::Secondary,
                        "Badge"
                    )
                </div>
            )
        )
    };
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("UI — argentum-ui seam")
                argentum_ui::page_description(
                    "Per-app contract: styles.css (@import tailwindcss + Tokens + @source for app and argentum-ui) + build.rs (argentum_ui::tailwind_build) + tailwind::stylesheet!() and Geist font in layout. This page proves it with a card, button, and badge — all Token-only, no raw colors."
                )
            )
            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Card + Button + Badge"
                </h2>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "argentum_ui::card(\n    card_header(card_title(\"Beautiful card\"))\n    card_content(button(variant: Primary, \"Primary\"))\n)"
                )
                (card_view)
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
