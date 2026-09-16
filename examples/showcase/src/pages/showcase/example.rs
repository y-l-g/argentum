use topcoat::{
    Result,
    view::{Child, View, component, view},
};

/// One showcase example: title, prose, the minimal snippet, and the rendered
/// result.
///
/// Owning the vertical rhythm here is deliberate (GH #151 §6): title,
/// description, code, and demo are always a `gap-4` apart, so a demo control
/// can never end up flush against the paragraph above it.
///
/// ```ignore
/// example(
///     title: "Section"
///     description: "Titled container."
///     code: "Section::new(\"Account\").schema(Text::new(\"hello\"))"
///     (section)
/// )
/// ```
#[component]
pub async fn example(
    #[into]
    #[default]
    title: String,
    #[into]
    #[default]
    description: String,
    #[into]
    #[default]
    code: String,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <section
            class="flex flex-col gap-4 rounded-xl border border-border bg-background p-6 shadow-sm"
        >
            <h2 class="text-lg font-semibold tracking-tight text-foreground">
                (title)
            </h2>
            if !description.is_empty() {
                <p class="text-sm text-muted-foreground">(description)</p>
            }
            if !code.is_empty() {
                argentum_ui::code_block(lang: "rust", code: code)
            }
            <div
                class="flex flex-col gap-4 rounded-lg border border-border bg-background p-4"
            >
                (child)
            </div>
        </section>
    })
}
