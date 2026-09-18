//! Layout containers — `Section`, `Group`, `Grid`, `Repeater`, `Tabs`, `Wizard`.
//!
//! The compositional seams for form layout; each holds an optional child
//! `Schema` rendered through the tree walk.

use argentum_ui::{
    FieldLegendVariant, card, card_content, card_header, card_title, field_error as ui_field_error,
    field_group as ui_field_group, field_legend as ui_field_legend, field_set as ui_field_set,
};
use topcoat::{Result, context::Cx, view::*};

use super::Schema;
use super::tree::{IntoSchema, RenderSource};

/// Section — titled container with an optional child `Schema`.
///
/// The single customization seam for form layout in v1: additive `class` is
/// allowed on the `card` container only (narrow seam, no per-field `attrs`).
/// This keeps Token editing in `styles.css` as the primary theming mechanism.
#[derive(Debug)]
pub struct Section {
    title: String,
    pub(crate) children: Option<Schema>,
    extra_class: Option<String>,
}

impl Section {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            children: None,
            extra_class: None,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    /// Additive `class` hook on the `card` container (narrow seam).
    /// Merged via `class!` against Token classes, never replacing them.
    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.extra_class = Some(class.into());
        self
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        let title = self.title.clone();
        let extra = self.extra_class.clone();
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                    card_content((child_view))
                )
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                )
            }
            .boxed())
        }
    }
}

/// Group — unlabelled container, useful for grouping fields.
#[derive(Debug)]
pub struct Group {
    pub(crate) children: Option<Schema>,
}

impl Default for Group {
    fn default() -> Self {
        Self::new()
    }
}

impl Group {
    pub fn new() -> Self {
        Self { children: None }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! { cx => ui_field_group((child_view)) }.boxed())
        } else {
            Ok(view! { cx => ui_field_group() }.boxed())
        }
    }
}

/// Grid — column container. `cols` is 1..12.
#[derive(Debug)]
pub struct Grid {
    cols: u8,
    pub(crate) children: Option<Schema>,
}

impl Grid {
    pub fn new(cols: u8) -> Self {
        Self {
            cols: cols.clamp(1, 12),
            children: None,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        // Static literals for Tailwind scanner — `format!("grid grid-cols-{}")` would be
        // purged because Tailwind only sees literal substrings. See ADR-0007 / T2.
        let class: &'static str = match self.cols {
            1 => "grid grid-cols-1 gap-4",
            2 => "grid grid-cols-2 gap-4",
            3 => "grid grid-cols-3 gap-4",
            4 => "grid grid-cols-4 gap-4",
            5 => "grid grid-cols-5 gap-4",
            6 => "grid grid-cols-6 gap-4",
            7 => "grid grid-cols-7 gap-4",
            8 => "grid grid-cols-8 gap-4",
            9 => "grid grid-cols-9 gap-4",
            10 => "grid grid-cols-10 gap-4",
            11 => "grid grid-cols-11 gap-4",
            _ => "grid grid-cols-12 gap-4",
        };
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! { cx => <div class=(class)>(child_view)</div> }.boxed())
        } else {
            Ok(view! { cx => <div class=(class)></div> }.boxed())
        }
    }
}

/// Repeater — nested Schema repeated as a group (in-memory for v1, no DB array).
///
/// v1 honesty (GH #73): this is a single-entry group, not a multi-row repeater —
/// one titled card with its nested schema once, no add/remove UI, no JS, no
/// indexed field names (`tags[0]`). Indexed multi-entry semantics, per-entry
/// validation, and hydration via split/join or a real relation are deferred.
/// `required` means "the inner fields must not all be empty" and its error is
/// keyed by label and rendered inline (GH #78).
#[derive(Debug)]
pub struct Repeater {
    pub(crate) label: String,
    pub(crate) children: Option<Schema>,
    pub(crate) required: bool,
}

impl Repeater {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: None,
            required: false,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        let title = self.label.clone();
        let required = self.required;
        // Own error lives under the label key (see `walk_repeater_absence`).
        // Field errors key by field name; repeaters have no field name yet, so the
        // label is the only stable key until repeaters become field-bound (GH #78).
        let own_errors: &[String] = match source {
            RenderSource::Static { errors, .. } | RenderSource::Live { errors, .. } => {
                errors.get(&self.label).map(|v| v.as_slice()).unwrap_or(&[])
            }
        };
        let has_error = !own_errors.is_empty();
        let error_text = own_errors.first().cloned().unwrap_or_default();
        let container_class = if has_error {
            "ac-field ac-field--error rounded-md border border-border p-4"
        } else {
            "ac-field rounded-md border border-border p-4"
        };
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! {
                cx =>
                ui_field_set(
                    attrs: attributes! { class=(container_class) },
                    ui_field_legend(
                        variant: FieldLegendVariant::Label,
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    )
                    <div class="grid gap-4">(child_view)</div>
                    ui_field_error(
                        attrs: attributes! { class="ac-error" aria-live="polite" },
                        (error_text)
                    )
                )
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                ui_field_set(
                    attrs: attributes! { class=(container_class) },
                    ui_field_legend(
                        variant: FieldLegendVariant::Label,
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    )
                    ui_field_error(
                        attrs: attributes! { class="ac-error" aria-live="polite" },
                        (error_text)
                    )
                )
            }
            .boxed())
        }
    }
}

/// Shared no-JS chrome for the `Tabs` / `Wizard` grouping containers (GH #73):
/// both render as a bordered column until their step/tab scripts land, so the
/// markup lives in one place and the public types stay distinct seams.
#[derive(Debug)]
pub(crate) struct Container {
    pub(crate) children: Option<Schema>,
}

impl Container {
    fn new() -> Self {
        Self { children: None }
    }

    fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4">
                    (child_view)
                </div>
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                <div class="flex flex-col gap-4 border border-border rounded-md p-4"></div>
            }
            .boxed())
        }
    }
}

/// Tabs — layout primitive for tabbed content (in-memory for v1, no JS).
///
/// Static `div` grouping for v1 (GH #73): looks like tabs, behaves as stacked
/// sections until tab JS lands. Documented, not a placeholder bug.
#[derive(Debug)]
pub struct Tabs(pub(crate) Container);

impl Tabs {
    pub fn new() -> Self {
        Self(Container::new())
    }

    pub fn schema(self, children: impl IntoSchema) -> Self {
        Self(self.0.schema(children))
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        self.0.render_source(cx, source).await
    }
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

/// Wizard — step-based layout (in-memory for v1, no JS).
///
/// Static `div` grouping for v1 (GH #73): looks like steps, behaves as stacked
/// sections until step JS lands. Documented, not a placeholder bug.
#[derive(Debug)]
pub struct Wizard(pub(crate) Container);

impl Wizard {
    pub fn new() -> Self {
        Self(Container::new())
    }

    pub fn schema(self, children: impl IntoSchema) -> Self {
        Self(self.0.schema(children))
    }

    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        self.0.render_source(cx, source).await
    }
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::{Cx, CxTestBuilder};

    use crate::schema::{Schema, Text, TextInput};

    use super::*;

    fn cx() -> Cx {
        CxTestBuilder::new().build()
    }
    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        #[unique]
        email: String,
    }

    #[tokio::test]
    async fn text_input_inside_section_and_grid() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()).required(),
            TextInput::r#for(DummyUser::fields().email()).email(),
        ))));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // Section now renders as card with Token classes
        assert!(
            html.contains("border-border"),
            "missing card border in {html}"
        );
        assert!(html.contains("bg-card"), "missing card bg in {html}");
        assert!(html.contains("shadow-sm"), "missing card shadow in {html}");
        assert!(html.contains("grid grid-cols-2"), "missing grid in {html}");
        assert!(
            html.contains("data-slot=\"field\""),
            "missing field in {html}"
        );
    }

    #[tokio::test]
    async fn section_renders_title_and_child() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Text::new("hello")));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Account"), "missing title in {html}");
        assert!(html.contains("hello"), "missing child in {html}");
        // Section now renders as card
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing card chrome in {html}"
        );
        assert!(
            html.contains("px-6"),
            "missing card header/content padding in {html}"
        );
        assert!(
            html.contains("font-semibold"),
            "missing card title in {html}"
        );
    }

    #[tokio::test]
    async fn group_renders_children() {
        let cx = cx();
        let schema = Schema::new(Group::new().schema(Text::new("inside group")));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("inside group"), "missing child in {html}");
        assert!(
            html.contains("@container/field-group"),
            "missing field_group markup in {html}"
        );
    }

    #[tokio::test]
    async fn grid_renders_with_cols_and_children() {
        let cx = cx();
        let schema = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("grid"), "missing grid class in {html}");
        assert!(
            html.contains("grid-cols-2"),
            "missing cols class grid-cols-2 in {html}"
        );
        assert!(html.contains("gap-4"), "missing gap-4 in {html}");
        assert!(
            html.contains(">a<") || html.contains("text-foreground\">a"),
            "missing a in {html}"
        );
        assert!(
            html.contains(">b<") || html.contains("text-foreground\">b"),
            "missing b in {html}"
        );
    }

    #[tokio::test]
    async fn tabs_and_wizard_render_children_in_one_container() {
        let cx = cx();
        let cases = [
            (
                Schema::new(Tabs::new().schema(Text::new("tabbed"))),
                "tabbed",
            ),
            (
                Schema::new(Wizard::new().schema(Text::new("stepped"))),
                "stepped",
            ),
        ];
        for (schema, child) in cases {
            let html = schema
                .render(&cx)
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx);
            assert!(html.contains(child), "missing {child} in {html}");
            assert_eq!(
                html.matches("border border-border rounded-md p-4").count(),
                1,
                "expected one shared container wrapper in {html}"
            );
        }
    }

    #[tokio::test]
    async fn tabs_and_wizard_validate_and_render_fields_end_to_end() {
        // GH #136 extension: Tabs/Wizard had no end-to-end coverage — the only
        // tabs test was the UI demo `?tab=`, and the showcase wires neither
        // as Schema containers. This pins that required inputs inside the
        // containers validate and render with values.
        let cx = cx();
        for make in [
            |input: TextInput| Schema::new(Tabs::new().schema(input)),
            |input: TextInput| Schema::new(Wizard::new().schema(input)),
        ] {
            let schema = make(TextInput::r#for(DummyUser::fields().name()).required());
            let errors = schema.validate(&HashMap::new());
            assert!(
                errors.contains_key("name"),
                "empty submit must fail the inner required input, got {errors:?}"
            );
            let mut values = HashMap::new();
            values.insert("name".to_string(), "Ada".to_string());
            let errors = schema.validate(&values);
            assert!(errors.is_empty(), "filled submit must pass, got {errors:?}");
            let html = schema
                .render_with(&cx, &values, &errors)
                .await
                .unwrap()
                .single()
                .await
                .unwrap()
                .render(&cx);
            assert!(
                html.contains("value=\"Ada\""),
                "container must render the field value, got {html}"
            );
        }
    }

    #[tokio::test]
    async fn nested_grid_inside_section() {
        let cx = cx();
        let schema = Schema::new(
            Section::new("Outer")
                .schema(Grid::new(2).schema((Text::new("left"), Text::new("right")))),
        );
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Outer"), "missing outer title in {html}");
        assert!(html.contains("left"), "missing left in {html}");
        assert!(html.contains("right"), "missing right in {html}");
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing section card in {html}"
        );
        assert!(html.contains("grid-cols-2"), "missing grid in {html}");
    }

    #[tokio::test]
    async fn repeater_required_error_renders_inline() {
        let cx = cx();
        // Single-entry repeater (GH #78): the required error is keyed by label
        // until repeaters become field-bound.
        let schema = Schema::new(
            Repeater::new("Tags")
                .required()
                .schema(TextInput::r#for(DummyUser::fields().name()).label("Tag")),
        );
        let values = HashMap::new();
        let errors = schema.validate(&values);
        assert!(
            errors.contains_key("Tags"),
            "required repeater must produce a label-keyed error, got {errors:?}"
        );
        let html = schema
            .render_with(&cx, &values, &errors)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("Tags is required"),
            "repeater error must reach the HTML, got {html}"
        );
        // Same inline error contract as TextInput: ac-error slot + live region.
        assert!(
            html.contains("ac-error") && html.contains("text-destructive"),
            "missing inline error slot in {html}"
        );
        assert!(
            html.contains("aria-live=\"polite\""),
            "missing aria-live in {html}"
        );
        // Non-empty inner value clears the error.
        let mut filled = HashMap::new();
        filled.insert("name".to_string(), "rust".to_string());
        let errors = schema.validate(&filled);
        assert!(
            !errors.contains_key("Tags"),
            "filled repeater must pass, got {errors:?}"
        );
    }

    /// An optional Repeater with a `required` inner input must not fail an
    /// empty submit (GH #147): group-empty means "absent". A `required`
    /// repeater answers an empty submit with exactly one label-keyed error,
    /// and a partially filled optional group still enforces inner `required`.
    #[test]
    fn optional_repeater_with_required_inner_allows_empty_group() {
        // Two inner inputs so "partially filled" is expressible: a required
        // text field and an optional email field.
        let optional = Schema::new(
            Repeater::new("Tags").schema((
                TextInput::r#for(DummyUser::fields().name())
                    .required()
                    .label("Tag"),
                TextInput::r#for(DummyUser::fields().email())
                    .optional()
                    .label("Note"),
            )),
        );
        let required = Schema::new(
            Repeater::new("Tags").required().schema((
                TextInput::r#for(DummyUser::fields().name())
                    .required()
                    .label("Tag"),
                TextInput::r#for(DummyUser::fields().email())
                    .optional()
                    .label("Note"),
            )),
        );

        // Empty submit: the optional group validates clean...
        let errors = optional.validate(&HashMap::new());
        assert!(
            errors.is_empty(),
            "optional repeater with empty group must validate clean, got {errors:?}"
        );
        // ...the required group answers with exactly one label-keyed error —
        // the inner input's own required error is suppressed with the absent
        // group, so the label carries the whole story (GH #147).
        let errors = required.validate(&HashMap::new());
        assert_eq!(
            errors.len(),
            1,
            "required repeater + empty submit must yield one error, got {errors:?}"
        );
        assert_eq!(
            errors.get("Tags").map(|errs| errs.as_slice()),
            Some(["Tags is required".to_string()].as_slice()),
            "the label-keyed error is the only one, got {errors:?}"
        );

        // A partially filled optional group counts as present: inner
        // `required` fires for the empty input, not for the optional one.
        let mut partial = HashMap::new();
        partial.insert("email".to_string(), "a@b.c".to_string());
        let errors = optional.validate(&partial);
        assert!(
            errors.contains_key("name"),
            "a partially filled group enforces inner required, got {errors:?}"
        );
        assert!(
            !errors.contains_key("email"),
            "the optional inner input stays optional, got {errors:?}"
        );
    }

    /// A `required` repeater nested inside an all-empty OPTIONAL group is
    /// suppressed with it (GH #147): an untouched outer group means nothing
    /// inside it was intended, so the inner label error must not fire.
    #[test]
    fn required_repeater_inside_absent_optional_group_is_suppressed() {
        let schema = Schema::new(
            Repeater::new("Outer").schema((
                TextInput::r#for(DummyUser::fields().name()).required(),
                Repeater::new("Inner")
                    .required()
                    .schema(TextInput::r#for(DummyUser::fields().email()).required()),
            )),
        );
        // Empty submit: the outer group is absent, so the inner required
        // repeater fires no error at all.
        let errors = schema.validate(&HashMap::new());
        assert!(
            errors.is_empty(),
            "an untouched optional outer group must suppress nested required repeaters, got {errors:?}"
        );
        // With the outer group present (a value anywhere in its subtree),
        // the inner required repeater enforces.
        let mut present = HashMap::new();
        present.insert("name".to_string(), "rust".to_string());
        let errors = schema.validate(&present);
        assert!(
            errors.contains_key("Inner"),
            "a present outer group enforces the inner required repeater, got {errors:?}"
        );
    }
}
