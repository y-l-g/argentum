use std::collections::HashMap;

use argentum_core::{
    Grid, Group, Schema, Section, Text, TextInput,
    notification::{LiveToast, live_toast},
};
use topcoat::{
    Result,
    context::Cx,
    router::page,
    runtime::{Event, Signal, procedure, shard, signal},
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

/// Validate the demo form (GH #154 §4).
///
/// The rules stay [`Schema::validate`] over the same declaration; the result
/// crosses as `(name_error, email_error)` (empty = valid) because only the
/// shared vocabulary crosses to the client. The submit handler writes it into
/// the form's error signals.
#[procedure]
pub async fn validate_schema_form(name: String, email: String) -> Result<(String, String)> {
    let values = HashMap::from([("name".to_string(), name), ("email".to_string(), email)]);
    let errors = validation_schema().validate(&values);
    let first = |field: &str| {
        errors
            .get(field)
            .and_then(|errs| errs.first())
            .cloned()
            .unwrap_or_default()
    };
    Ok((first("name"), first("email")))
}

/// The live-validation form (GH #154 §4): a shard, so a submit re-renders
/// just this region — no navigation, no scroll jump, no PRG.
///
/// The field and error signals are page-owned (passed in as handles), so the
/// submit handler writes the procedure result into the same signals the shard
/// reads: an error signal change re-renders the fields, and the valid path
/// writes the page's live toast, which the shell mounts.
/// The module carries the lint allow: the shard's arity is its dependency
/// list (one signal per field and toast slot), and the macro expands the face
/// past clippy's default (same pattern as `panel::shard_body`).
#[allow(clippy::too_many_arguments)]
mod live_form_shard {
    use super::*;

    #[shard]
    pub async fn live_validation_form(
        cx: &Cx,
        name: Signal<String>,
        email: Signal<String>,
        name_error: Signal<String>,
        email_error: Signal<String>,
        toast_status: Signal<String>,
        toast_title: Signal<String>,
        toast_description: Signal<String>,
        toast_serial: Signal<u64>,
    ) -> Result<impl View> {
        render_live_validation_form(
            cx,
            &name,
            &email,
            &name_error,
            &email_error,
            &toast_status,
            &toast_title,
            &toast_description,
            &toast_serial,
        )
        .await
    }
}
pub use live_form_shard::live_validation_form;

#[allow(clippy::too_many_arguments)]
async fn render_live_validation_form<'a>(
    cx: &'a Cx,
    name: &Signal<String>,
    email: &Signal<String>,
    name_error: &Signal<String>,
    email_error: &Signal<String>,
    toast_status: &Signal<String>,
    toast_title: &Signal<String>,
    toast_description: &Signal<String>,
    toast_serial: &Signal<u64>,
) -> Result<BoxView<'a>> {
    let schema = validation_schema();
    // The error signals are the shard's dependencies: the submit handler
    // writes the procedure result into them and the fields re-render. Values
    // arrive with the rerun (the signals travel as shard arguments), so they
    // are not read here.
    let errors: HashMap<String, Vec<String>> =
        [("name", name_error.get()), ("email", email_error.get())]
            .into_iter()
            .filter(|(_, error)| !error.is_empty())
            .map(|(field, error)| (field.to_string(), vec![error]))
            .collect();
    let live_values = HashMap::from([
        ("name".to_string(), name.clone()),
        ("email".to_string(), email.clone()),
    ]);
    let fields = schema.render_live_with(cx, &live_values, &errors).await?;
    let name = name.clone();
    let email = email.clone();
    let name_error = name_error.clone();
    let email_error = email_error.clone();
    let toast_status = toast_status.clone();
    let toast_title = toast_title.clone();
    let toast_description = toast_description.clone();
    let toast_serial = toast_serial.clone();
    Ok(view! {
        cx =>
        <form
            class="flex flex-col gap-4"
            @submit=$(async |e: Event| {
                e.prevent_default();
                let errs = validate_schema_form(name.get(), email.get()).await;
                let valid = if errs.0.is_empty() { errs.1.is_empty() } else { false };
                name_error.set(errs.0);
                email_error.set(errs.1);
                if valid {
                    toast_status.set("success".to_owned());
                    toast_title.set("Validated".to_owned());
                    toast_description.set("Both fields passed.".to_owned());
                    toast_serial.increment();
                }
            })
        >
            (fields)
            <div>
                argentum_ui::button(
                    variant: argentum_ui::ButtonVariant::Primary,
                    attrs: attributes! { type="submit" },
                    "Submit"
                )
            </div>
        </form>
    }
    .boxed())
}

#[page("/admin/showcase/schema")]
async fn schema_showcase(cx: &Cx) -> Result<impl View> {
    render_schema_page(cx).await
}

/// One renderer for the page: every example pairs title → snippet → result.
async fn render_schema_page<'a>(cx: &'a Cx) -> Result<BoxView<'a>> {
    // The live-validation form's signals: the page owns them so both the
    // shard (args) and the submit handler (captured in the shard view) share
    // the same handles. The toast ones mount through the shell's toaster.
    let LiveToast {
        status: toast_status,
        title: toast_title,
        description: toast_description,
        serial: toast_serial,
    } = live_toast(cx);
    let name = signal(cx, String::new);
    let email = signal(cx, String::new);
    let name_error = signal(cx, String::new);
    let email_error = signal(cx, String::new);
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
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(
                argentum_ui::page_title("Schema")
                argentum_ui::page_description(
                    "Unified layout primitive for forms and infolists. Text + Section/Group/Grid compose via IntoSchema tuples and render through view! — and every example pairs its snippet with the rendered result. The live-validation form submits without a reload."
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
                description: "Validation rules compose on top of the schema default; `.email()` rejects malformed addresses inline. Submit the live-validation form below with `not-an-email` to watch the rule fire under the field.",
                code: "TextInput::for(User::fields().email()).email()",
                (input_email)
            )

            example(
                title: "Live validation",
                description: "The same schema drives a real form: a #[procedure] validates the values through Schema::validate, the submit handler writes its per-field errors into signals, and a shard re-renders the fields in place — errors appear under their field (ac-field--error + ac-error) with no navigation and no PRG. Valid submits toast in place.",
                code: "// #[procedure] validate_schema_form(name, email) -> (name_err, email_err)\nlet errs = validate_schema_form(name.get(), email.get()).await;\nname_error.set(errs.0); // the error slots update in place\nif errs.0.is_empty() && errs.1.is_empty() { /* toast */ }",
                live_validation_form(
                    name: $(name),
                    email: $(email),
                    name_error: $(name_error),
                    email_error: $(email_error),
                    toast_status: $(toast_status),
                    toast_title: $(toast_title),
                    toast_description: $(toast_description),
                    toast_serial: $(toast_serial)
                )
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
