use std::collections::HashMap;

use argentum_core::{
    Grid, Group, Notification, Schema, Section, Text, TextInput, csrf,
    notification::set_notification,
};
use topcoat::{
    Result,
    context::Cx,
    router::{content::Form, error::see_other, page},
    view::{BoxView, View, ViewExt, attributes, view},
};

use super::example::example;
use crate::models::User;

/// The form behind the live-validation example: required is inferred from the
/// non-nullable columns, so no `.required()` call appears anywhere.
fn validation_schema() -> Schema {
    Schema::new((
        TextInput::r#for(User::fields().name()),
        TextInput::r#for(User::fields().email()).email(),
    ))
}

#[page("/admin/showcase/schema")]
async fn schema_showcase(cx: &Cx) -> Result<impl View> {
    render_schema_page(cx, &HashMap::new(), &HashMap::new()).await
}

/// Validate the demo form: invalid submits re-render the page with inline
/// errors; a valid submit flashes a toast and redirects (PRG).
#[page(POST "/admin/showcase/schema")]
async fn schema_validate_post(
    cx: &Cx,
    Form(values): Form<HashMap<String, String>>,
) -> Result<impl View> {
    csrf::verify(cx, &values)?;
    let errors = validation_schema().validate(&values);
    if errors.is_empty() {
        set_notification(
            cx,
            Notification::success("Validated")
                .description("Both fields passed, so the handler redirected (PRG)."),
        );
        return Err(see_other("/admin/showcase/schema").into());
    }
    render_schema_page(cx, &values, &errors).await
}

/// One renderer for GET and the invalid POST, so the page has a single shape.
async fn render_schema_page<'a>(
    cx: &'a Cx,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    // Every example renders live, in the order title → snippet → result.
    let text = Schema::new(Text::new("hello")).render(cx).await?;
    let input_required = Schema::new(TextInput::r#for(User::fields().name()))
        .render(cx)
        .await?;
    let input_optional = Schema::new(TextInput::r#for(User::fields().name()).optional())
        .render(cx)
        .await?;
    let input_email = Schema::new(TextInput::r#for(User::fields().email()).email())
        .render(cx)
        .await?;
    let validation_form = validation_schema().render_with(cx, values, errors).await?;
    let section = Schema::new(Section::new("Account").schema(Text::new("hello")))
        .render(cx)
        .await?;
    let group = Schema::new(Group::new().schema(Text::new("inside group")))
        .render(cx)
        .await?;
    // Grids are laid out around fields; the bordered inputs make the columns
    // visible (GH #151 §8).
    let grid_2 = Schema::new(
        Grid::new(2).schema((
            TextInput::r#for(User::fields().name())
                .optional()
                .label("Left"),
            TextInput::r#for(User::fields().email())
                .optional()
                .label("Right"),
        )),
    )
    .render(cx)
    .await?;
    let nested = Schema::new(
        Section::new("Outer").schema(
            Grid::new(2).schema((
                TextInput::r#for(User::fields().name())
                    .optional()
                    .label("Left"),
                TextInput::r#for(User::fields().email())
                    .optional()
                    .label("Right"),
            )),
        ),
    )
    .render(cx)
    .await?;
    let composed = Schema::new((
        Section::new("Account").schema(Text::new("a")),
        Grid::new(2).schema((
            TextInput::r#for(User::fields().name())
                .optional()
                .label("Left"),
            TextInput::r#for(User::fields().email())
                .optional()
                .label("Right"),
        )),
    ))
    .render(cx)
    .await?;
    let csrf_token = csrf::ensure_token(cx);
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Schema")
                argentum_ui::page_description(
                    "Unified layout primitive for forms and infolists. Text + Section/Group/Grid compose via IntoSchema tuples and render through view! — and every example pairs its snippet with the rendered result."
                )
            )

            example(
                title: "Text (leaf)",
                description: "A plain text leaf. Typed fields replace it in real forms.",
                code: "Schema::new(Text::new(\"hello\"))",
                (text)
            )

            example(
                title: "TextInput",
                description: "Bound to a Toasty lens; required is inferred from the column. A non-nullable String is required by default, so the asterisk needs no call. (Nullable Option<String> lenses do not bind TextInput yet, GH #115 — they will infer optional when they do.)",
                code: "TextInput::for(User::fields().name())\n// non-nullable → required, no .required() call",
                (input_required)
            )

            example(
                title: "TextInput — optional",
                description: "`.optional()` is the explicit opt-out: the field keeps its value but drops the required rule and the red asterisk.",
                code: "TextInput::for(User::fields().name()).optional()",
                (input_optional)
            )

            example(
                title: "TextInput — email",
                description: "Validation rules compose on top of the schema default; `.email()` rejects malformed addresses inline.",
                code: "TextInput::for(User::fields().email()).email()",
                (input_email)
            )

            example(
                title: "Live validation",
                description: "The same schema drives a real form: submitting it validates the values, re-renders the fields with inline errors when invalid, and redirects with a toast when valid.",
                code: "let errors = schema.validate(&values);\nschema.render_with(cx, &values, &errors) // inline errors\n// valid: set_notification(...); Err(see_other(\"/admin/showcase/schema\").into())",
                // `novalidate` lets the server render the inline errors:
                // without it the browser's native `required` blocks the
                // submit before the round-trip this demo exists to show.
                <form
                    method="post"
                    action="/admin/showcase/schema"
                    novalidate=""
                    class="flex flex-col gap-4"
                >
                    <input type="hidden" name=(csrf::FIELD_NAME) value=(csrf_token)>
                    (validation_form)
                    <div>
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Primary,
                            attrs: attributes! { type="submit" },
                            "Submit"
                        )
                    </div>
                </form>
            )

            example(
                title: "Section",
                description: "Titled container around a schema.",
                code: "Section::new(\"Account\").schema(Text::new(\"hello\"))",
                (section)
            )

            example(
                title: "Group",
                description: "Unlabelled container.",
                code: "Group::new().schema(Text::new(\"inside group\"))",
                (group)
            )

            example(
                title: "Grid",
                description: "Column container, 1–12 columns (larger values clamp to 12). The demo fields are render-only, so `.optional()` just drops the asterisk; the columns are visible because real inputs fill them.",
                code: "Grid::new(2).schema((\n    TextInput::for(User::fields().name()).optional().label(\"Left\"),\n    TextInput::for(User::fields().email()).optional().label(\"Right\"),\n))",
                (grid_2)
            )

            example(
                title: "Grid in Section",
                description: "Nested layouts compose: a two-column field grid inside a titled section.",
                code: "Section::new(\"Outer\").schema(Grid::new(2).schema((\n    TextInput::for(User::fields().name()).optional().label(\"Left\"),\n    TextInput::for(User::fields().email()).optional().label(\"Right\"),\n)))",
                (nested)
            )

            example(
                title: "Composition",
                description: "Tuples of IntoSchema become one schema — a section and a grid compose in place.",
                code: "Schema::new((\n    Section::new(\"Account\").schema(Text::new(\"a\")),\n    Grid::new(2).schema((\n        TextInput::for(User::fields().name()).optional().label(\"Left\"),\n        TextInput::for(User::fields().email()).optional().label(\"Right\"),\n    )),\n))",
                (composed)
            )

            <p>
                <a href="/admin/showcase" class="text-sm text-primary hover:underline">
                    "← back to showcase"
                </a>
                " | "
                <a href="/admin" class="text-sm text-primary hover:underline">
                    "→ admin list"
                </a>
            </p>
        )
    }
    .boxed())
}
