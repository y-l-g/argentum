//! Layout containers — `Section`, `Group`, `Grid`, `Repeater`, `Tabs`.
//!
//! The compositional seams for form layout; each holds an optional child
//! `Schema` rendered through the tree walk.

use argentum_ui::{
    FieldLegendVariant, card, card_content, card_header, card_title, field_error as ui_field_error,
    field_group as ui_field_group, field_legend as ui_field_legend, field_set as ui_field_set,
};
use topcoat::{Result, context::Cx, view::*};

use super::Schema;
use super::tree::{IntoSchema, Mode, RenderSource};

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
        // A view renders the group's label over its children's values (GH #187):
        // a required group is a statement about a submit that cannot happen
        // here, so no `*`, no `aria-invalid`, no error slot.
        if let RenderSource::Static {
            mode: Mode::View, ..
        } = source
        {
            let child_view = match &self.children {
                Some(schema) => Some(schema.render_source(cx, source).await?),
                None => None,
            };
            return Ok(view! {
                cx =>
                ui_field_set(
                    attrs: attributes! { class="ac-field rounded-md border border-border p-4" },
                    ui_field_legend(
                        variant: FieldLegendVariant::Label,
                        attrs: attributes! {},
                        (title)
                    )
                    if let Some(child_view) = child_view {
                        <div class="grid gap-4">(child_view)</div>
                    }
                )
            }
            .boxed());
        }
        let required = self.required;
        // Own error lives under the label key (see `walk_repeater_absence`).
        // Field errors key by field name; repeaters have no field name yet, so the
        // label is the only stable key until repeaters become field-bound (GH #78).
        // `ignores_errors` is the one place "view mode has no errors" lives, so a
        // second layout that reads errors cannot forget it.
        let own_errors: &[String] = if source.ignores_errors() {
            &[]
        } else {
            match source {
                RenderSource::Static { errors, .. } => {
                    errors.get(&self.label).map(|v| v.as_slice()).unwrap_or(&[])
                }
            }
        };
        let has_error = !own_errors.is_empty();
        let error_text = own_errors.first().cloned().unwrap_or_default();
        // The group's error is described by the fieldset, so it needs an id to
        // be referenced by; the label is the key (GH #78), and a label is not
        // usable as one (ids cannot carry whitespace).
        let error_id = repeater_error_id(&self.label);
        let container_class = if has_error {
            "ac-field ac-field--error rounded-md border border-border p-4"
        } else {
            "ac-field rounded-md border border-border p-4"
        };
        // `field_legend` (unlike `field_label`) has no invalid state of its
        // own, so the group colors its legend when it is invalid.
        let legend_class = if has_error { "text-destructive" } else { "" };
        if let Some(schema) = &self.children {
            let child_view = schema.render_source(cx, source).await?;
            Ok(view! {
                cx =>
                ui_field_set(
                    attrs: attributes! {
                        class=(container_class)
                        data-invalid=(has_error.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    },
                    ui_field_legend(
                        variant: FieldLegendVariant::Label,
                        attrs: attributes! { class=(legend_class) },
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    )
                    <div class="grid gap-4">(child_view)</div>
                    if has_error {
                        ui_field_error(
                            attrs: attributes! {
                                id=(error_id.clone())
                                class="ac-error"
                                aria-live="polite"
                            },
                            (error_text)
                        )
                    }
                )
            }
            .boxed())
        } else {
            Ok(view! {
                cx =>
                ui_field_set(
                    attrs: attributes! {
                        class=(container_class)
                        data-invalid=(has_error.then_some("true"))
                        aria-invalid=(if has_error { "true" } else { "false" })
                        aria-describedby=(has_error.then_some(error_id.clone()))
                    },
                    ui_field_legend(
                        variant: FieldLegendVariant::Label,
                        attrs: attributes! { class=(legend_class) },
                        (title)
                        if required {
                            <span class="text-destructive" aria-hidden="true">"*"</span>
                        }
                    )
                    if has_error {
                        ui_field_error(
                            attrs: attributes! {
                                id=(error_id.clone())
                                class="ac-error"
                                aria-live="polite"
                            },
                            (error_text)
                        )
                    }
                )
            }
            .boxed())
        }
    }
}

/// The DOM id of a repeater's error node.
///
/// Repeaters are keyed by their label until they become field-bound
/// (GH #78), and an id may not carry the label's whitespace, so the label is
/// slugged: ASCII alphanumerics lowercased, every other run collapsed to one
/// `-`.
fn repeater_error_id(label: &str) -> String {
    let mut slug = String::with_capacity(label.len());
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    format!("{}-error", slug.trim_matches('-'))
}

/// Shared no-JS chrome for the grouping container (GH #73): renders as a
/// bordered column until the tab script lands, so the markup lives in one
/// place and the public type stays the seam.
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::{Cx, CxTestBuilder};

    use crate::schema::{Schema, TextInput};

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
        let schema = Schema::new(
            Section::new("Account").schema(TextInput::r#for(DummyUser::fields().name())),
        );
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Account"), "missing title in {html}");
        assert!(
            html.contains("name=\"name\""),
            "missing child field in {html}"
        );
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
        let schema = Schema::new(
            Group::new().schema(TextInput::r#for(DummyUser::fields().name()).label("Inside group")),
        );
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Inside group"), "missing child in {html}");
        assert!(
            html.contains("@container/field-group"),
            "missing field_group markup in {html}"
        );
    }

    #[tokio::test]
    async fn grid_renders_with_cols_and_children() {
        let cx = cx();
        let schema = Schema::new(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()),
            TextInput::r#for(DummyUser::fields().email()),
        )));
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
            html.contains("name=\"name\""),
            "missing first child in {html}"
        );
        assert!(
            html.contains("name=\"email\""),
            "missing second child in {html}"
        );
    }

    #[tokio::test]
    async fn tabs_render_children_in_one_container() {
        let cx = cx();
        let schema = Schema::new(
            Tabs::new().schema(TextInput::r#for(DummyUser::fields().name()).label("Tabbed")),
        );
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Tabbed"), "missing child in {html}");
        assert_eq!(
            html.matches("border border-border rounded-md p-4").count(),
            1,
            "expected one shared container wrapper in {html}"
        );
    }

    #[tokio::test]
    async fn tabs_validate_and_render_fields_end_to_end() {
        // GH #136 extension: the container had no end-to-end coverage — the
        // only tabs test was the UI demo `?tab=`, and the showcase wires no
        // layout block as a Schema container here. This pins that required
        // inputs inside the container validate and render with values.
        let cx = cx();
        let schema = Schema::new(
            Tabs::new().schema(TextInput::r#for(DummyUser::fields().name()).required()),
        );
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

    #[tokio::test]
    async fn nested_grid_inside_section() {
        let cx = cx();
        let schema = Schema::new(Section::new("Outer").schema(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()).label("Left"),
            TextInput::r#for(DummyUser::fields().email()).label("Right"),
        ))));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Outer"), "missing outer title in {html}");
        assert!(html.contains("Left"), "missing left in {html}");
        assert!(html.contains("Right"), "missing right in {html}");
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
        // Same inline error contract as TextInput, wired to the group: the
        // fieldset carries the invalid state and describes itself with the
        // error node's id, and the legend is colored (`field_legend`, unlike
        // `field_label`, has no invalid state of its own).
        assert!(
            html.contains("data-invalid=\"true\"")
                && html.contains("aria-invalid=\"true\"")
                && html.contains("aria-describedby=\"tags-error\""),
            "repeater must expose its invalid state in {html}"
        );
        assert!(
            html.contains("id=\"tags-error\"") && html.contains("ac-error"),
            "missing inline error slot in {html}"
        );
        assert!(
            html.contains("aria-live=\"polite\""),
            "missing aria-live in {html}"
        );
        let legend = html
            .split("<legend")
            .nth(1)
            .and_then(|rest| rest.split('>').next())
            .unwrap_or_default();
        assert!(
            legend.contains("text-destructive"),
            "invalid repeater must color its legend, got {legend}"
        );
        // Non-empty inner value clears the error.
        let mut filled = HashMap::new();
        filled.insert("name".to_string(), "rust".to_string());
        let errors = schema.validate(&filled);
        assert!(
            !errors.contains_key("Tags"),
            "filled repeater must pass, got {errors:?}"
        );
        // A valid group carries no invalid state and no error node.
        let valid_html = schema
            .render_with(&cx, &filled, &errors)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !valid_html.contains("data-invalid")
                && !valid_html.contains("role=\"alert\"")
                && !valid_html.contains("tags-error"),
            "a valid repeater must not render invalid state in {valid_html}"
        );
    }

    #[test]
    fn repeater_error_ids_slug_the_label() {
        // Ids cannot carry the label's whitespace (GH #78 keys the error by
        // label until repeaters are field-bound).
        assert_eq!(repeater_error_id("Tags"), "tags-error");
        assert_eq!(
            repeater_error_id("Shipping Address"),
            "shipping-address-error"
        );
        assert_eq!(
            repeater_error_id("  Billing / Info  "),
            "billing-info-error"
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
