use argentum_core::{Grid, Group, Schema, Section, Text, TextInput};
use topcoat::{
    Result,
    context::Cx,
    router::page,
    view::{View, view},
};

use crate::models::User;

#[page("/admin/showcase/schema")]
async fn schema_showcase(cx: &Cx) -> Result<impl View> {
    // Live renders for each variant — keep code minimal and matching snippets.
    let text = Schema::new(Text::new("hello")).render(cx).await?;
    let input_name = Schema::new(TextInput::r#for(User::fields().name()))
        .render(cx)
        .await?;
    let input_required = Schema::new(TextInput::r#for(User::fields().name()).required())
        .render(cx)
        .await?;
    let input_email = Schema::new(TextInput::r#for(User::fields().email()).required().email())
        .render(cx)
        .await?;
    let input_tuple = Schema::new((
        TextInput::r#for(User::fields().name()),
        TextInput::r#for(User::fields().email()),
    ))
    .render(cx)
    .await?;
    let input_in_layout = Schema::new(Section::new("Account").schema(Grid::new(2).schema((
        TextInput::r#for(User::fields().name()).required(),
        TextInput::r#for(User::fields().email()).email(),
    ))))
    .render(cx)
    .await?;
    // Validation demo — static inline errors (empty → required, bad email)
    let err_required = TextInput::r#for(User::fields().name())
        .required()
        .validate("")
        .join(", ");
    let err_email = TextInput::r#for(User::fields().email())
        .email()
        .validate("not-an-email")
        .join(", ");
    let err_none = TextInput::r#for(User::fields().name())
        .required()
        .validate("hello")
        .join(", ");
    let section_empty = Schema::new(Section::new("Account")).render(cx).await?;
    let section_with_child = Schema::new(Section::new("Account").schema(Text::new("hello")))
        .render(cx)
        .await?;
    let group = Schema::new(Group::new().schema(Text::new("inside group")))
        .render(cx)
        .await?;
    let grid_1 = Schema::new(Grid::new(1).schema(Text::new("one col")))
        .render(cx)
        .await?;
    let grid_2 = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))))
        .render(cx)
        .await?;
    let grid_12 = Schema::new(Grid::new(12).schema(Text::new("12 cols")))
        .render(cx)
        .await?;
    let grid_clamped = Schema::new(Grid::new(15).schema(Text::new("15 → clamped to 12")))
        .render(cx)
        .await?;
    let nested = Schema::new(
        Section::new("Outer").schema(Grid::new(2).schema((Text::new("left"), Text::new("right")))),
    )
    .render(cx)
    .await?;
    let composed = Schema::new((
        Section::new("A").schema(Text::new("a")),
        Group::new().schema(Text::new("b")),
    ))
    .render(cx)
    .await?;
    let empty = Schema::empty().render(cx).await?;

    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Schema")
                argentum_ui::page_description(
                    "Unified layout primitive for forms/infolists. Text + Section/Group/Grid compose via IntoSchema tuples and render through view!."
                )
            )

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Text (leaf)"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Placeholder leaf — will become typed fields (TextInput, Select...)."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Schema::new(Text::new(\"hello\"))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (text)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "TextInput (typed field)"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Bound to a Toasty lens — `TextInput::for(User::fields().name())` fails if column missing. Variants: plain, required, email."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "TextInput::for(User::fields().name())\nTextInput::for(User::fields().name()).required()\nTextInput::for(User::fields().email()).required().email()\nSchema::new((TextInput::for(User::fields().name()), TextInput::for(User::fields().email())))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3 class="text-sm font-medium text-foreground">"name"</h3>
                    (input_name)
                    <h3 class="text-sm font-medium text-foreground">"required"</h3>
                    (input_required)
                    <h3 class="text-sm font-medium text-foreground">"email"</h3>
                    (input_email)
                    <h3 class="text-sm font-medium text-foreground">
                        "tuple (name, email)"
                    </h3>
                    (input_tuple)
                    <h3 class="text-sm font-medium text-foreground">
                        "in Section+Grid"
                    </h3>
                    (input_in_layout)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Validation (inline)"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "`required` and `email` return per-field errors; empty required → error, bad email → error, valid → no error."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "TextInput::for(User::fields().name()).required().validate(\"\") // [\"Name is required\"]\nTextInput::for(User::fields().email()).email().validate(\"bad\") // [\"Email must be a valid email\"]"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <p class="text-sm text-muted-foreground">
                        "required empty: "
                        <span class="text-sm text-destructive">(err_required)</span>
                    </p>
                    <p class="text-sm text-muted-foreground">
                        "email bad: "
                        <span class="text-sm text-destructive">(err_email)</span>
                    </p>
                    <p class="text-sm text-muted-foreground">
                        "valid: "
                        (if err_none.is_empty() { "— no errors" } else { &err_none })
                    </p>
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Section"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Titled container. Empty vs with child."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Section::new(\"Account\")\nSection::new(\"Account\").schema(Text::new(\"hello\"))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3 class="text-sm font-medium text-foreground">"empty"</h3>
                    (section_empty)
                    <h3 class="text-sm font-medium text-foreground">"with child"</h3>
                    (section_with_child)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Group"
                </h2>
                <p class="text-sm text-muted-foreground">"Unlabelled container."</p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Group::new().schema(Text::new(\"inside group\"))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    (group)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Grid"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Column container, cols 1..12 (clamped). Variants: 1, 2, 12, 15→12, nested in Section."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Grid::new(1).schema(Text::new(\"one col\"))\nGrid::new(2).schema((Text::new(\"a\"), Text::new(\"b\")))\nGrid::new(12).schema(Text::new(\"12 cols\"))\nGrid::new(15).schema(Text::new(\"15 → clamped to 12\")) // grid-cols-12\nSection::new(\"Outer\").schema(Grid::new(2).schema((Text::new(\"left\"), Text::new(\"right\"))))"
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3 class="text-sm font-medium text-foreground">"1 col"</h3>
                    (grid_1)
                    <h3 class="text-sm font-medium text-foreground">"2 cols"</h3>
                    (grid_2)
                    <h3 class="text-sm font-medium text-foreground">"12 cols"</h3>
                    (grid_12)
                    <h3 class="text-sm font-medium text-foreground">"15 clamped"</h3>
                    (grid_clamped)
                    <h3 class="text-sm font-medium text-foreground">
                        "nested Grid in Section"
                    </h3>
                    (nested)
                </div>
            </section>

            <section
                class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
            >
                <h2 class="text-lg font-semibold tracking-tight text-foreground">
                    "Composition"
                </h2>
                <p class="text-sm text-muted-foreground">
                    "Tuples of IntoSchema become one Schema. Empty renders nothing."
                </p>
                argentum_ui::code_block(
                    lang: "rust",
                    code: "Schema::new((Section::new(\"A\").schema(Text::new(\"a\")), Group::new().schema(Text::new(\"b\"))))\nSchema::empty() // renders \"\""
                )
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3 class="text-sm font-medium text-foreground">
                        "(Section, Group)"
                    </h3>
                    (composed)
                    <h3 class="text-sm font-medium text-foreground">"empty"</h3>
                    (empty)
                    <span class="text-sm text-muted-foreground">
                        "(empty — no ac-* classes)"
                    </span>
                </div>
            </section>

            <p>
                <a href="/admin/showcase">"← back to showcase"</a>
                " | "
                <a href="/admin">"→ admin list"</a>
            </p>
        )
    })
}
