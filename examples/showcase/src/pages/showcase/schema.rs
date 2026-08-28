use argentum_core::{Grid, Group, Schema, Section, Text, TextInput};
use topcoat::{Result, context::Cx, router::page, view::view};

use crate::models::User;

#[page("/admin/showcase/schema")]
async fn schema_showcase(cx: &Cx) -> Result {
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

    view! {
        <div class="flex flex-col gap-8 max-w-4xl mx-auto p-6">
            <h1>"Schema"</h1>
            <p>"Unified layout primitive for forms/infolists. Text + Section/Group/Grid compose via IntoSchema tuples and render through view!."</p>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Text (leaf)"</h2>
                <p>"Placeholder leaf — will become typed fields (TextInput, Select...)."</p>
                <pre><code>"Schema::new(Text::new(\"hello\"))"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">(text)</div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"TextInput (typed field)"</h2>
                <p>"Bound to a Toasty lens — `TextInput::for(User::fields().name())` fails if column missing. Variants: plain, required, email."</p>
                <pre><code>"TextInput::for(User::fields().name())\nTextInput::for(User::fields().name()).required()\nTextInput::for(User::fields().email()).required().email()\nSchema::new((TextInput::for(User::fields().name()), TextInput::for(User::fields().email())))"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3>"name"</h3> (input_name)
                    <h3>"required"</h3> (input_required)
                    <h3>"email"</h3> (input_email)
                    <h3>"tuple (name, email)"</h3> (input_tuple)
                    <h3>"in Section+Grid"</h3> (input_in_layout)
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Validation (inline)"</h2>
                <p>"`required` and `email` return per-field errors; empty required → error, bad email → error, valid → no error."</p>
                <pre><code>"TextInput::for(User::fields().name()).required().validate(\"\") // [\"Name is required\"]\nTextInput::for(User::fields().email()).email().validate(\"bad\") // [\"Email must be a valid email\"]"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">
                    <p>"required empty: " <span class="text-sm text-destructive">(err_required)</span></p>
                    <p>"email bad: " <span class="text-sm text-destructive">(err_email)</span></p>
                    <p>"valid: " (if err_none.is_empty() { "— no errors" } else { &err_none })</p>
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Section"</h2>
                <p>"Titled container. Empty vs with child."</p>
                <pre><code>"Section::new(\"Account\")\nSection::new(\"Account\").schema(Text::new(\"hello\"))"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3>"empty"</h3> (section_empty)
                    <h3>"with child"</h3> (section_with_child)
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Group"</h2>
                <p>"Unlabelled container."</p>
                <pre><code>"Group::new().schema(Text::new(\"inside group\"))"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">(group)</div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Grid"</h2>
                <p>"Column container, cols 1..12 (clamped). Variants: 1, 2, 12, 15→12, nested in Section."</p>
                <pre><code>"Grid::new(1).schema(Text::new(\"one col\"))\nGrid::new(2).schema((Text::new(\"a\"), Text::new(\"b\")))\nGrid::new(12).schema(Text::new(\"12 cols\"))\nGrid::new(15).schema(Text::new(\"15 → clamped to 12\")) // grid-cols-12\nSection::new(\"Outer\").schema(Grid::new(2).schema((Text::new(\"left\"), Text::new(\"right\"))))"</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3>"1 col"</h3> (grid_1)
                    <h3>"2 cols"</h3> (grid_2)
                    <h3>"12 cols"</h3> (grid_12)
                    <h3>"15 clamped"</h3> (grid_clamped)
                    <h3>"nested Grid in Section"</h3> (nested)
                </div>
            </section>

            <section class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                <h2>"Composition"</h2>
                <p>"Tuples of IntoSchema become one Schema. Empty renders nothing."</p>
                <pre><code>"Schema::new((Section::new(\"A\").schema(Text::new(\"a\")), Group::new().schema(Text::new(\"b\"))))\nSchema::empty() // renders \"\""</code></pre>
                <div class="rounded-lg border border-border bg-background p-4">
                    <h3>"(Section, Group)"</h3> (composed)
                    <h3>"empty"</h3> (empty) <span class="text-sm text-muted-foreground">"(empty — no ac-* classes)"</span>
                </div>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a>" | " <a href="/admin">"→ admin list"</a></p>
        </div>
    }
}
