use argentum_ui::components::primitives::{
    accordion::{accordion, accordion_content, accordion_item, accordion_trigger},
    avatar::{AvatarSize, avatar, avatar_fallback, avatar_image},
    breadcrumb::{
        breadcrumb, breadcrumb_item, breadcrumb_link, breadcrumb_list, breadcrumb_page,
        breadcrumb_separator,
    },
    checkbox::checkbox,
    dropdown_menu::{
        dropdown_menu, dropdown_menu_content, dropdown_menu_item, dropdown_menu_label,
        dropdown_menu_separator, dropdown_menu_sub, dropdown_menu_sub_content,
        dropdown_menu_sub_trigger, dropdown_menu_trigger,
    },
    kbd::{kbd, kbd_group},
    progress::progress,
    radio_group::{radio_group, radio_group_item},
    select::select,
    skeleton::skeleton,
    spinner::spinner,
    switch::switch,
    tabs::{tabs, tabs_content, tabs_list, tabs_trigger},
    textarea::textarea,
    toggle::{ToggleKind, ToggleSize, toggle, toggle_group},
};
use topcoat::{
    Result,
    context::Cx,
    icon::icon,
    router::{page, query_params},
    view::{View, ViewExt, attributes, view},
};

use super::example::example;

#[query_params]
struct UiQuery {
    tab: Option<String>,
}

/// Inline avatar art for the demo — a tiny SVG data URI, so the page never
/// requests a missing asset and `avatar_image` really renders (GH #151 §5).
const AVATAR_ART: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'%3E%3Crect width='48' height='48' fill='%23a5b4fc'/%3E%3C/svg%3E";

#[page("/admin/showcase/ui")]
async fn ui_showcase(cx: &Cx) -> Result<impl View> {
    // The tabs primitive is server state: the page renders the panel the URL
    // names, so the demo actually switches (and survives a reload).
    let tab = query_params::<UiQuery>(cx)
        .ok()
        .and_then(|q| q.tab.clone())
        .unwrap_or_else(|| "overview".to_string());
    // ErrorState's action is a plain view; build it once, outside the demo.
    let retry = view! { cx => <a href="/admin/users">"Retry"</a> }.boxed();
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("UI — argentum-ui seam")
                argentum_ui::page_description(
                    "Every component family argentum-ui exports: primitives mirrored from topcoat-ui-registry plus the owned composites. All Token-only, no raw colors. The per-app contract is one styles.css (@import tailwindcss + Tokens + @source for app and argentum-ui) + a 3-line build.rs."
                )
            )

            example(
                title: "Card",
                description: "Header, title, description, content and footer compose the shadcn card.",
                code: "card(\n    card_header(card_title(\"Beautiful card\") card_description(\"...\"))\n    card_content(...)\n    card_footer(...)\n)",
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
                        <div class="mt-4 flex flex-wrap items-center gap-2">
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
                    argentum_ui::card_footer(
                        <span class="text-xs text-muted-foreground">"card_footer"</span>
                    )
                )
            )

            example(
                title: "Button",
                description: "Five variants and four sizes; `button_variants` styles anchors and summaries the same way.",
                code: "button(variant: Primary, \"Primary\")\nbutton(variant: Ghost, \"Ghost\")\nbutton(size: Icon, icon(data: icons::SEARCH, ...))",
                <div class="flex flex-wrap items-center gap-2">
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Primary,
                        "Primary"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Secondary,
                        "Secondary"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Outline,
                        "Outline"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Ghost,
                        "Ghost"
                    )
                    argentum_ui::button(
                        variant: argentum_ui::ButtonVariant::Destructive,
                        "Destructive"
                    )
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    argentum_ui::button(size: argentum_ui::ButtonSize::Sm, "Small")
                    argentum_ui::button(size: argentum_ui::ButtonSize::Lg, "Large")
                    argentum_ui::button(
                        size: argentum_ui::ButtonSize::Icon,
                        attrs: attributes! { aria-label="Search" },
                        icon(
                            data: argentum_ui::icons::SEARCH,
                            attrs: attributes! { class="size-4" }
                        )
                    )
                    <a
                        href="/admin/users"
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Outline,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Link button"
                    </a>
                    argentum_ui::button(attrs: attributes! { disabled="" }, "Disabled")
                </div>
            )

            example(
                title: "Badge & Alert",
                description: "Badges label status inline; alerts carry a title and a description block.",
                code: "badge(variant: Destructive, \"Failed\")\nalert(variant: Destructive, alert_title(\"Build failed\") alert_description(\"...\"))",
                <div class="flex flex-wrap items-center gap-2">
                    argentum_ui::badge(
                        variant: argentum_ui::BadgeVariant::Primary,
                        "Primary"
                    )
                    argentum_ui::badge(
                        variant: argentum_ui::BadgeVariant::Secondary,
                        "Secondary"
                    )
                    argentum_ui::badge(
                        variant: argentum_ui::BadgeVariant::Outline,
                        "Outline"
                    )
                    argentum_ui::badge(
                        variant: argentum_ui::BadgeVariant::Destructive,
                        "Failed"
                    )
                </div>
                <div class="flex flex-col gap-3">
                    argentum_ui::alert(
                        argentum_ui::alert_title("Heads up")
                        argentum_ui::alert_description(
                            "Neutral alerts describe state without alarm."
                        )
                    )
                    argentum_ui::alert(
                        variant: argentum_ui::AlertVariant::Destructive,
                        argentum_ui::alert_title("Build failed")
                        argentum_ui::alert_description(
                            "Destructive alerts use the destructive Token."
                        )
                    )
                </div>
            )

            example(
                title: "Form controls",
                description: "Input, textarea, select, checkbox, switch, radio group, toggle group, label and kbd — native controls styled with Tokens.",
                code: "input(attrs: attributes! { type=\"email\" placeholder=\"you@example.com\" })\nselect(attrs: attributes! { name=\"region\" }, <option>...</option>)\ncheckbox(attrs: attributes! { id=\"terms\" checked=\"\" })\nswitch(attrs: attributes! { id=\"airplane\" checked=\"\" })",
                <div class="grid gap-4 sm:grid-cols-2">
                    <div class="grid gap-1.5">
                        argentum_ui::label(
                            attrs: attributes! { for="ui-email" },
                            "Email"
                        )
                        argentum_ui::input(
                            attrs: attributes! { id="ui-email" type="email" placeholder="you@example.com" }
                        )
                    </div>
                    <div class="grid gap-1.5">
                        argentum_ui::label(
                            attrs: attributes! { for="ui-region" },
                            "Region"
                        )
                        select(
                            attrs: attributes! { id="ui-region" name="region" },
                            <option>"eu-central-1"</option>
                            <option selected="">"us-east-1"</option>
                        )
                    </div>
                    <div class="flex items-center gap-2">
                        checkbox(attrs: attributes! { id="ui-terms" checked="" })
                        argentum_ui::label(
                            attrs: attributes! { for="ui-terms" },
                            "Accept terms"
                        )
                    </div>
                    <div class="flex items-center gap-2">
                        switch(attrs: attributes! { id="ui-airplane" checked="" })
                        argentum_ui::label(
                            attrs: attributes! { for="ui-airplane" },
                            "Airplane mode"
                        )
                    </div>
                    <div class="flex items-center gap-4">
                        radio_group(
                            <div class="flex items-center gap-2">
                                radio_group_item(
                                    attrs: attributes! {
                                        id="ui-weekly"
                                        name="ui-billing"
                                        value="weekly"
                                        checked=""
                                    }
                                )
                                argentum_ui::label(
                                    attrs: attributes! { for="ui-weekly" },
                                    "Weekly"
                                )
                            </div>
                            <div class="flex items-center gap-2">
                                radio_group_item(
                                    attrs: attributes! { id="ui-monthly" name="ui-billing" value="monthly" }
                                )
                                argentum_ui::label(
                                    attrs: attributes! { for="ui-monthly" },
                                    "Monthly"
                                )
                            </div>
                        )
                    </div>
                    <div class="flex items-center gap-2">
                        toggle_group(
                            toggle(
                                kind: ToggleKind::Exclusive,
                                size: ToggleSize::Sm,
                                attrs: attributes! { name="ui-range" value="day" checked="" },
                                "Day"
                            )
                            toggle(
                                kind: ToggleKind::Exclusive,
                                size: ToggleSize::Sm,
                                attrs: attributes! { name="ui-range" value="week" },
                                "Week"
                            )
                        )
                    </div>
                    <div class="grid gap-1.5 sm:col-span-2">
                        argentum_ui::label(
                            attrs: attributes! { for="ui-notes" },
                            "Notes"
                        )
                        textarea(
                            attrs: attributes! { id="ui-notes" name="notes" placeholder="Tell us more" }
                        )
                    </div>
                    <div class="grid gap-1.5">
                        argentum_ui::label(
                            attrs: attributes! { for="ui-disabled" },
                            "Disabled"
                        )
                        argentum_ui::input(
                            attrs: attributes! { id="ui-disabled" value="Read only" disabled="" }
                        )
                    </div>
                    <div class="flex items-center gap-3">
                        <span class="text-sm text-muted-foreground">"Shortcut"</span>
                        kbd_group(
                            kbd("Ctrl")
                            kbd("K")
                        )
                    </div>
                </div>
            )

            example(
                title: "Table",
                description: "The table primitive wraps the scroll container, caption, header, body, footer, rows and cells.",
                code: "table(\n    table_caption(\"Environments\")\n    table_header(table_row(table_head(\"Environment\") table_head(\"Status\")))\n    table_body(\n        table_row(table_cell(\"production\") table_cell(badge(variant: Primary, \"Live\")))\n    )\n    table_footer(table_row(table_cell(attrs: attributes! { colspan=\"3\" }, \"2 environments\")))\n)",
                argentum_ui::table(
                    argentum_ui::table_caption("Environments")
                    argentum_ui::table_header(
                        argentum_ui::table_row(
                            argentum_ui::table_head("Environment")
                            argentum_ui::table_head("Status")
                            argentum_ui::table_head("Updated")
                        )
                    )
                    argentum_ui::table_body(
                        argentum_ui::table_row(
                            argentum_ui::table_cell("production")
                            argentum_ui::table_cell(
                                argentum_ui::badge(
                                    variant: argentum_ui::BadgeVariant::Primary,
                                    "Live"
                                )
                            )
                            argentum_ui::table_cell("2m ago")
                        )
                        argentum_ui::table_row(
                            argentum_ui::table_cell("staging")
                            argentum_ui::table_cell(
                                argentum_ui::badge(
                                    variant: argentum_ui::BadgeVariant::Secondary,
                                    "Degraded"
                                )
                            )
                            argentum_ui::table_cell("1h ago")
                        )
                    )
                    argentum_ui::table_footer(
                        argentum_ui::table_row(
                            argentum_ui::table_cell(
                                attrs: attributes! { colspan="3" },
                                "2 environments"
                            )
                        )
                    )
                )
            )

            example(
                title: "Avatar, breadcrumb, pagination & separator",
                description: "Navigation and identity pieces: avatars fall back to initials, breadcrumbs chain links, separators divide. Pagination is a static preview here — every control points at the live list rather than a paged page.",
                code: "avatar(size: AvatarSize::Lg, avatar_image(...) avatar_fallback(\"AL\"))\nbreadcrumb(breadcrumb_list(breadcrumb_item(breadcrumb_link(...)) breadcrumb_separator() ...))\npagination(pagination_content(pagination_item(pagination_link(active: true, ...))))\nseparator(orientation: SeparatorOrientation::Vertical)",
                <div class="flex flex-wrap items-center gap-4">
                    avatar(
                        size: AvatarSize::Sm,
                        avatar_image(attrs: attributes! { src=(AVATAR_ART) alt="" })
                        avatar_fallback("AL")
                    )
                    // No image: the initials fallback stands alone.
                    avatar(avatar_fallback("AL"))
                    avatar(
                        size: AvatarSize::Lg,
                        avatar_image(attrs: attributes! { src=(AVATAR_ART) alt="" })
                        avatar_fallback("AL")
                    )
                    <div class="flex h-8 items-center gap-4">
                        argentum_ui::separator(
                            orientation: argentum_ui::SeparatorOrientation::Vertical
                        )
                        <span class="text-sm text-muted-foreground">
                            "vertical separator"
                        </span>
                        argentum_ui::separator(
                            orientation: argentum_ui::SeparatorOrientation::Vertical
                        )
                    </div>
                </div>
                breadcrumb(
                    breadcrumb_list(
                        breadcrumb_item(
                            breadcrumb_link(
                                attrs: attributes! { href="/admin/showcase" },
                                "Showcase"
                            )
                        )
                        breadcrumb_separator()
                        breadcrumb_item(
                            breadcrumb_link(
                                attrs: attributes! { href="/admin/showcase/ui" },
                                "UI"
                            )
                        )
                        breadcrumb_separator()
                        breadcrumb_item(breadcrumb_page("UI"))
                    )
                )
                argentum_ui::pagination(
                    argentum_ui::pagination_content(
                        argentum_ui::pagination_item(
                            argentum_ui::pagination_previous(
                                attrs: attributes! { href="/admin/users" }
                            )
                        )
                        argentum_ui::pagination_item(
                            argentum_ui::pagination_link(
                                active: true,
                                attrs: attributes! { href="/admin/users" },
                                "2"
                            )
                        )
                        argentum_ui::pagination_item(
                            argentum_ui::pagination_link(
                                attrs: attributes! { href="/admin/users" },
                                "3"
                            )
                        )
                        argentum_ui::pagination_item(argentum_ui::pagination_ellipsis())
                        argentum_ui::pagination_item(
                            argentum_ui::pagination_link(
                                attrs: attributes! { href="/admin/users" },
                                "9"
                            )
                        )
                        argentum_ui::pagination_item(
                            argentum_ui::pagination_next(
                                attrs: attributes! { href="/admin/users" }
                            )
                        )
                    )
                )
            )

            example(
                title: "Progress, skeleton & spinner",
                description: "Loading and progress shapes: determinate and indeterminate progress, pulse skeletons, and the spinner.",
                code: "progress(value: 62.0)\nprogress() // indeterminate\nskeleton(attrs: attributes! { class=\"h-4 w-32\" })\nspinner()",
                <div class="flex flex-col gap-4">
                    progress(value: 62.0, attrs: attributes! { aria-label="62%" })
                    progress(attrs: attributes! { aria-label="Indeterminate" })
                    <div class="flex items-center gap-3">
                        skeleton(attrs: attributes! { class="size-10" })
                        <div class="flex flex-col gap-2">
                            skeleton(attrs: attributes! { class="h-4 w-48" })
                            skeleton(attrs: attributes! { class="h-4 w-32" })
                        </div>
                    </div>
                    <div class="flex items-center gap-3 text-sm text-muted-foreground">
                        spinner()
                        "Loading…"
                    </div>
                </div>
            )

            example(
                title: "Accordion, dropdown & tabs",
                description: "Native details for accordion and dropdown (its items are a visual preview); tabs are server state — the triggers reload the page with the panel the URL names, landing back on the demo.",
                code: "accordion(accordion_item(accordion_trigger(\"Question\") accordion_content(\"Answer\")))\ndropdown_menu(\n    dropdown_menu_trigger(attrs: attributes! { class=(button_variants(Outline, Md)) }, \"Actions\" icon(...))\n    dropdown_menu_content(dropdown_menu_label(\"Edit\") dropdown_menu_item(\"Rename\") dropdown_menu_sub(...))\n)\ntabs(attrs: attributes! { id=\"ui-tabs\" }, tabs_list(tabs_trigger(active: tab == \"overview\", attrs: attributes! { href=\"?tab=overview#ui-tabs\" }, \"Overview\")) tabs_content(...))",
                accordion(
                    accordion_item(
                        attrs: attributes! { name="ui-faq" },
                        accordion_trigger("Is this real?")
                        accordion_content(
                            "Yes — native details/summary, no JavaScript needed."
                        )
                    )
                    accordion_item(
                        attrs: attributes! { name="ui-faq" },
                        accordion_trigger("Can I open two?")
                        accordion_content(
                            "Drop the shared name attribute and each item toggles on its own."
                        )
                    )
                )
                dropdown_menu(
                    dropdown_menu_trigger(
                        attrs: attributes! {
                            class=(argentum_ui::button_variants(
                                argentum_ui::ButtonVariant::Outline,
                                argentum_ui::ButtonSize::Md,
                            ))
                        },
                        "Actions"
                        icon(
                            data: argentum_ui::icons::ARROW_DOWN,
                            attrs: attributes! { class="size-4 group-open:rotate-180" }
                        )
                    )
                    dropdown_menu_content(
                        dropdown_menu_label("Edit")
                        dropdown_menu_item("Rename")
                        dropdown_menu_item("Duplicate")
                        dropdown_menu_separator()
                        dropdown_menu_sub(
                            dropdown_menu_sub_trigger("Share")
                            dropdown_menu_sub_content(
                                dropdown_menu_item("Copy link")
                                dropdown_menu_item("Email")
                            )
                        )
                        dropdown_menu_separator()
                        dropdown_menu_item(
                            attrs: attributes! { class="text-destructive" },
                            "Delete"
                        )
                    )
                )
                tabs(
                    attrs: attributes! { id="ui-tabs" },
                    tabs_list(
                        tabs_trigger(
                            active: tab == "overview",
                            attrs: attributes! { href="?tab=overview#ui-tabs" },
                            "Overview"
                        )
                        tabs_trigger(
                            active: tab == "activity",
                            attrs: attributes! { href="?tab=activity#ui-tabs" },
                            "Activity"
                        )
                    )
                    tabs_content(
                        if tab == "activity" {
                            "Activity panel — rendered from ?tab=activity."
                        } else {
                            "Overview panel — rendered from ?tab=overview (the default)."
                        }
                    )
                )
            )

            example(
                title: "Overlays: dialog, sheet, toast",
                description: "Heavy overlays are driven by the shell assets rather than inline demos here: the Dialog page has a live alert_dialog and four toast triggers, and the sidebar's mobile navigation uses the sheet. Tooltip and hover_card are included below because their code is part of the surface.",
                code: "alert_dialog(open: open, dialog_content(dialog_header(dialog_title(\"Delete user?\")) dialog_footer(...)))\nsheet(open: false, sheet_content(side: SheetSide::Left, ...))\ntoaster(toast(attrs: attributes! { data-type=\"success\" }, toast_icon(...) toast_content(toast_title(\"Created\")) toast_close()))\ntooltip(button(...) tooltip_content(\"Copy link\"))\nhover_card(<a href=\"/admin/users\">\"@ada\"</a> hover_card_content(...))",
                <div class="flex flex-wrap items-center gap-2">
                    <a
                        href="/admin/showcase/dialog?open=true"
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Outline,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Live alert_dialog"
                    </a>
                    <a
                        href="/admin/showcase/dialog"
                        class=(argentum_ui::button_variants(
                            argentum_ui::ButtonVariant::Outline,
                            argentum_ui::ButtonSize::Md,
                        ))
                    >
                        "Toast triggers (Dialog page)"
                    </a>
                </div>
                <p class="text-sm text-muted-foreground">
                    "Tooltip and hover_card cannot be demoed inside the shell: both key their bubble on an unnamed `.group` hover, and the sidebar provider tags the whole page `group` (GH #151). They need a named group in the registry first, so only their code is shown."
                </p>
            )

            example(
                title: "ErrorState",
                description: "The failed-load block: destructive accent, generic detail, and a retry action. Distinct from an empty state — no answer is not zero rows.",
                code: "error_state(\n    title: \"Couldn't load Users\",\n    detail: \"Something went wrong while loading the records.\",\n    action: Some(retry.into()),\n)",
                argentum_ui::error_state(
                    title: "Couldn't load Users",
                    detail: "Something went wrong while loading the records.",
                    action: Some(retry.into())
                )
            )

            example(
                title: "CodeBlock, Page, Sidebar & theme",
                description: "Composites the shell builds from. Page owns the max-width, padding and rhythm of every showcase page; code_block is server-highlighted syntect with a copy button; sidebar_provider + sidebar are the shell, shown here as a static menu preview; theme_init_script runs pre-paint.",
                code: "page(page_header(page_title(\"UI\")) page_content(...))\ncode_block(lang: \"rust\", code: \"// comments use the theme token\")\nsidebar_provider(sidebar(sidebar_menu(sidebar_menu_item(...))) sidebar_inset(...))\ntheme_init_script() // blocking <script> in <head>",
                <div class="flex flex-col gap-4">
                    argentum_ui::code_block(
                        lang: "rust",
                        code: "// Comments render with var(--muted-foreground) on both themes,\n// not the highlighter's low-contrast grey.\nlet table = Table::for::<User>(cx)\n    .columns(TextColumn::for(User::fields().name()).searchable());"
                    )
                    <div class="w-56 rounded-lg border border-border bg-background p-2">
                        argentum_ui::sidebar_menu(
                            argentum_ui::sidebar_menu_item(
                                argentum_ui::sidebar_menu_button(
                                    attrs: attributes! { href="/admin/users" },
                                    "Users"
                                )
                            )
                            argentum_ui::sidebar_menu_item(
                                argentum_ui::sidebar_menu_button(
                                    attrs: attributes! { href="/admin/posts" },
                                    "Posts"
                                )
                            )
                        )
                    </div>
                </div>
            )

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
