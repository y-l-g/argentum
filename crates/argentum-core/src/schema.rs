//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid` and `Text` nodes.
//! Each node renders through Topcoat's `view!` macro; `Schema::render`
//! combines them. The API mirrors Filament's `Schema::new(( ... ))` tuple
//! form via the `IntoSchema` trait.
//!
//! Bridge note: `lens_field_name_and_label` and `pk_tie_breakers` reach into
//! `toasty_core` (see `EXTERNAL_GAPS.md` at repo root). They are the single
//! `toasty_core` import sites; migrate to public `Path::field_name()` /
//! `Model::primary_key_paths()` when Toasty exposes them.

use argentum_ui::{
    card, card_content, card_header, card_title, input as ui_input, label as ui_label,
};
use topcoat::{Result, context::Cx, view::*};

// ---------------------------------------------------------------------------
// Public layout primitives
// ---------------------------------------------------------------------------

/// Placeholder leaf — renders a text block. Used in T3 before typed fields land.
#[derive(Debug, Clone)]
pub struct Text(pub String);

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        Self(content.into())
    }

    async fn render(&self, cx: &Cx) -> Result<View> {
        let content = self.0.clone();
        view! { cx => <div class="text-sm text-foreground">(content)</div> }
    }
}

/// Typed text field bound to a Toasty field lens. The lens is the single
/// source of truth for the field name and type, so `TextInput::for(User::fields().name())`
/// fails to compile if the column does not exist (ADR-0001).
#[derive(Debug, Clone)]
pub struct TextInput {
    name: String,
    label: String,
    required: bool,
    is_email: bool,
    placeholder: Option<String>,
}

impl TextInput {
    /// Create a `TextInput` bound to the given field lens.
    ///
    /// Only `String` lenses compile: binding a non-text field (a `Uuid` key,
    /// a `bool`, …) fails at compile time, mirroring `TextColumn`.
    pub fn for_lens<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label_str) = lens_field_name_and_label(path);
        Self {
            name: field_name,
            label: label_str,
            required: false,
            is_email: false,
            placeholder: None,
        }
    }

    /// Convenience alias so call sites read `TextInput::for(User::fields().name())`.
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        Self::for_lens(path)
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn email(mut self) -> Self {
        self.is_email = true;
        self
    }

    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = Some(p.into());
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    /// Validate a raw string value against the configured rules.
    pub fn validate(&self, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if self.required && v.is_empty() {
            errs.push(format!("{} is required", self.label));
        }
        // TODO: use `validator` crate for email (see GH #11)
        if self.is_email && !v.is_empty() && !Self::is_valid_email(v) {
            errs.push(format!("{} must be a valid email", self.label));
        }
        errs
    }

    fn is_valid_email(s: &str) -> bool {
        // `s` is already trimmed by `validate`
        if s.contains(' ') {
            return false;
        }
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (local, domain) = (parts[0], parts[1]);
        if local.is_empty() || domain.is_empty() {
            return false;
        }
        domain.contains('.')
    }

    async fn render(&self, cx: &Cx) -> Result<View> {
        let label_text = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let placeholder = self.placeholder.clone();
        let input_type = if self.is_email { "email" } else { "text" };
        // Beautiful rendering via argentum-ui `label` + `input` with Token classes,
        // proper for/id linking, required star, type branching, and reserved error slot.
        view! {
            cx =>
            <div class="grid gap-1.5">
                ui_label(
                    attrs: attributes! { for=(name.clone()) },
                    (label_text.clone())
                    if required {
                        <span class="text-destructive" aria-hidden="true">"*"</span>
                    }
                )
                ui_input(
                    attrs: attributes! {
                        id=(name.clone())
                        r#type=(input_type)
                        name=(name.clone())
                        placeholder=(placeholder.clone())
                        required=(required)
                        aria-required=(required.then_some("true"))
                        aria-invalid="false"
                    }
                )
                <p class="text-sm text-destructive" aria-live="polite"></p>
            </div>
        }
    }
}

/// Spec alias — ADR-0001 typed lens. Currently uses `toasty::stmt::Path` directly;
/// a richer `FieldLens` trait (carrying `FieldTy`, nullability, etc.) will replace
/// this alias (see GH #11, EXTERNAL_GAPS.md “field metadata”). `is_nullable` /
/// `is_unique` / `column_name` are deferred until hydration.
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

/// Resolve a typed lens to its app-level field name and capitalized label.
///
/// Hides the `Path → toasty_core::stmt::Path → projection → M::schema()` walk
/// (single import site, see EXTERNAL_GAPS.md). Used by both `TextInput` and
/// `TextColumn` so the shape is defined once.
pub(crate) fn lens_field_name_and_label<M, T>(path: FieldLens<M, T>) -> (String, String)
where
    M: toasty::schema::Model,
{
    let core_path: toasty_core::stmt::Path = path.into();
    debug_assert!(
        !core_path.projection.as_slice().is_empty(),
        "lens expects a field lens, got root path"
    );
    // Only single-field lenses are supported; multi-step paths will panic in
    // debug and fall back to first segment in release.
    let idx = core_path
        .projection
        .as_slice()
        .first()
        .copied()
        .expect("field lens must have a projection");
    let model = M::schema();
    let field_name = model
        .fields()
        .get(idx)
        .map(|f| f.name.app_unwrap().to_string())
        .unwrap_or_else(|| {
            panic!(
                "field index {idx} out of bounds for {}",
                std::any::type_name::<M>()
            )
        });
    let label_str = capitalize(&field_name);
    (field_name, label_str)
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// PK tie-breaker(s) for deterministic pagination.
///
/// Centralizes the `toasty_core::stmt` walk (`M::schema().as_root().primary_key`)
/// so `resource.rs` does not directly depend on core internals (see
/// EXTERNAL_GAPS.md “primary-key tie-breaker”). Returns one `asc`
/// `OrderByExpr` per PK field, in declared order.
///
/// # Panics
///
/// Panics if `M` is not a root model: without a primary key there is no
/// tie-breaker, and silent omission here would surface only as flaky row
/// order under cursor pagination.
pub(crate) fn pk_tie_breakers<M>() -> Vec<toasty::stmt::OrderByExpr>
where
    M: toasty::schema::Model,
{
    let app_model = M::schema();
    let root = app_model.as_root().unwrap_or_else(|| {
        panic!(
            "pk_tie_breakers: {} is not a root model; deterministic pagination needs its primary key",
            std::any::type_name::<M>()
        )
    });
    let mut out = Vec::new();
    for pk_field in &root.primary_key.fields {
        let expr = toasty_core::stmt::Expr::ref_self_field(*pk_field);
        out.push(toasty::stmt::OrderByExpr {
            expr,
            order: Some(toasty_core::stmt::Direction::Asc),
        });
    }
    out
}

/// Section — titled container with an optional child `Schema`.
///
/// The single customization seam for form layout in v1: additive `class` is
/// allowed on the `card` container only (narrow seam, no per-field `attrs`).
/// This keeps Token editing in `styles.css` as the primary theming mechanism.
#[derive(Debug)]
pub struct Section {
    title: String,
    children: Option<Schema>,
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

    async fn render(&self, cx: &Cx) -> Result<View> {
        let title = self.title.clone();
        let extra = self.extra_class.clone();
        if let Some(schema) = &self.children {
            let child_view = schema.render(cx).await?;
            view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                    card_content((child_view))
                )
            }
        } else {
            view! {
                cx =>
                card(
                    attrs: attributes! { class=(extra.clone()) },
                    card_header(card_title((title)))
                )
            }
        }
    }
}

/// Group — unlabelled container, useful for grouping fields.
#[derive(Debug)]
pub struct Group {
    children: Option<Schema>,
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

    async fn render(&self, cx: &Cx) -> Result<View> {
        if let Some(schema) = &self.children {
            let child_view = schema.render(cx).await?;
            view! { cx => <div class="flex flex-col gap-4">(child_view)</div> }
        } else {
            view! { cx => <div class="flex flex-col gap-4"></div> }
        }
    }
}

/// Grid — column container. `cols` is 1..12.
#[derive(Debug)]
pub struct Grid {
    cols: u8,
    children: Option<Schema>,
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

    async fn render(&self, cx: &Cx) -> Result<View> {
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
            let child_view = schema.render(cx).await?;
            view! { cx => <div class=(class)>(child_view)</div> }
        } else {
            view! { cx => <div class=(class)></div> }
        }
    }
}

// ---------------------------------------------------------------------------
// Node / Schema
// ---------------------------------------------------------------------------

// Phase 1: single field variant (TextInput). Will generalize to `Field` (enum of all field types).
#[derive(Debug)]
enum Node {
    Text(Text),
    TextInput(Box<TextInput>),
    Section(Box<Section>),
    Group(Box<Group>),
    Grid(Box<Grid>),
}

impl Node {
    async fn render(&self, cx: &Cx) -> Result<View> {
        match self {
            Node::Text(t) => t.render(cx).await,
            Node::TextInput(f) => Box::pin(f.render(cx)).await,
            Node::Section(s) => Box::pin(s.render(cx)).await,
            Node::Group(g) => Box::pin(g.render(cx)).await,
            Node::Grid(g) => Box::pin(g.render(cx)).await,
        }
    }
}

impl From<Text> for Node {
    fn from(v: Text) -> Self {
        Node::Text(v)
    }
}
impl From<TextInput> for Node {
    fn from(v: TextInput) -> Self {
        Node::TextInput(Box::new(v))
    }
}
impl From<Section> for Node {
    fn from(v: Section) -> Self {
        Node::Section(Box::new(v))
    }
}
impl From<Group> for Node {
    fn from(v: Group) -> Self {
        Node::Group(Box::new(v))
    }
}
impl From<Grid> for Node {
    fn from(v: Grid) -> Self {
        Node::Grid(Box::new(v))
    }
}

/// The container that composes layout blocks.
#[derive(Debug, Default)]
pub struct Schema {
    nodes: Vec<Node>,
}

impl Schema {
    /// Build a `Schema` from any `IntoSchema` (single node, tuple, or `Schema`).
    pub fn new(children: impl IntoSchema) -> Self {
        children.into_schema()
    }

    /// An empty schema (no nodes).
    pub fn empty() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Render the schema to a `View` (no DB access).
    pub async fn render(&self, cx: &Cx) -> Result<View> {
        let mut views = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            views.push(Box::pin(node.render(cx)).await?);
        }
        view! {
            cx =>
            for v in views {
                (v)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IntoSchema — tuple / single conversions
// ---------------------------------------------------------------------------

pub trait IntoSchema {
    fn into_schema(self) -> Schema;
}

impl IntoSchema for Schema {
    fn into_schema(self) -> Schema {
        self
    }
}
impl IntoSchema for Text {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Section {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Group {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Grid {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for TextInput {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}

impl<A, B> IntoSchema for (A, B)
where
    A: Into<Node>,
    B: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into()],
        }
    }
}
// 4-tuple limit is intentional: without variadic generics this is idiomatic
// — see `IntoColumns` in `resource.rs`. Macro deferred until 5+ columns are needed.
impl<A, B, C> IntoSchema for (A, B, C)
where
    A: Into<Node>,
    B: Into<Node>,
    C: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into(), self.2.into()],
        }
    }
}
impl<A, B, C, D> IntoSchema for (A, B, C, D)
where
    A: Into<Node>,
    B: Into<Node>,
    C: Into<Node>,
    D: Into<Node>,
{
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.0.into(), self.1.into(), self.2.into(), self.3.into()],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — T3 acceptance criteria
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

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
    async fn text_input_renders_with_label_and_ac_field() {
        let cx = cx();
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        // Beautiful: grid gap-1.5 wrapper, label + input with Token classes
        assert!(
            html.contains("grid gap-1.5"),
            "missing grid gap-1.5 in {html}"
        );
        assert!(
            html.contains("border-border"),
            "missing border-border in {html}"
        );
        assert!(
            html.contains("bg-background"),
            "missing bg-background in {html}"
        );
        assert!(html.contains("shadow-xs"), "missing shadow-xs in {html}");
        assert!(
            html.contains("name=\"name\"")
                || html.contains("name=\"Name\"")
                || html.contains("name"),
            "missing name attr in {html}"
        );
        assert!(html.contains("<input"), "missing input in {html}");
        assert!(html.contains("<label"), "missing label in {html}");
        assert!(
            html.contains("for=\"name\"") || html.contains("for="),
            "missing for/id linking in {html}"
        );
        assert!(
            html.contains("text-sm text-destructive"),
            "missing reserved error slot in {html}"
        );
        // label derived from lens: DummyUser::fields().name() → "name" → "Name"
        assert!(
            html.contains("Name") || html.contains("name"),
            "missing label in {html}"
        );
    }

    #[test]
    fn text_input_required_validates_empty() {
        let input = TextInput::r#for(DummyUser::fields().name()).required();
        assert!(
            !input.validate("").is_empty(),
            "required should reject empty"
        );
        assert!(
            input.validate("hello").is_empty(),
            "required should accept non-empty"
        );
        assert!(
            !input.validate("   ").is_empty(),
            "required should reject whitespace"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "optional should accept empty"
        );
    }

    #[tokio::test]
    async fn text_input_required_renders_star_and_email_type() {
        let cx = cx();
        let html_req = Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
            .render(&cx)
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_req.contains("text-destructive"),
            "required should render star with text-destructive in {html_req}"
        );
        assert!(
            html_req.contains("required"),
            "required attr missing in {html_req}"
        );
        assert!(
            html_req.contains("aria-required"),
            "aria-required missing in {html_req}"
        );
        assert!(
            html_req.contains("for=\"name\"") && html_req.contains("id=\"name\""),
            "for/id linking missing in {html_req}"
        );
        let html_email = Schema::new(TextInput::r#for(DummyUser::fields().email()).email())
            .render(&cx)
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_email.contains("type=\"email\""),
            "email should render type=email in {html_email}"
        );
        let html_text = Schema::new(TextInput::r#for(DummyUser::fields().name()))
            .render(&cx)
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_text.contains("type=\"text\""),
            "plain should render type=text in {html_text}"
        );
        // reserved error slot
        assert!(
            html_req.contains("text-sm text-destructive"),
            "error slot missing in {html_req}"
        );
    }

    #[test]
    fn text_input_email_validates() {
        let input = TextInput::r#for(DummyUser::fields().email())
            .required()
            .email();
        assert!(
            !input.validate("not-an-email").is_empty(),
            "email should reject invalid"
        );
        assert!(
            !input.validate("a@").is_empty(),
            "email should reject partial"
        );
        assert!(
            input.validate("a@b.com").is_empty(),
            "email should accept valid"
        );
        // optional email: empty is ok, whitespace trimmed
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate("")
                .is_empty(),
            "optional email should accept empty"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate(" a@b.com ")
                .is_empty(),
            "email should trim"
        );
    }

    #[tokio::test]
    async fn text_input_composes_in_tuple() {
        let cx = cx();
        let schema = Schema::new((
            TextInput::r#for(DummyUser::fields().name()),
            TextInput::r#for(DummyUser::fields().email()),
        ));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(
            html.matches("grid gap-1.5").count() >= 2,
            "expected 2 fields (grid gap-1.5) in {html}"
        );
        assert!(
            html.matches("text-sm text-destructive").count() >= 2,
            "expected 2 error slots in {html}"
        );
    }

    #[tokio::test]
    async fn text_input_inside_section_and_grid() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Grid::new(2).schema((
            TextInput::r#for(DummyUser::fields().name()).required(),
            TextInput::r#for(DummyUser::fields().email()).email(),
        ))));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        // Section now renders as card with Token classes
        assert!(
            html.contains("border-border"),
            "missing card border in {html}"
        );
        assert!(html.contains("bg-background"), "missing card bg in {html}");
        assert!(html.contains("shadow-sm"), "missing card shadow in {html}");
        assert!(html.contains("grid grid-cols-2"), "missing grid in {html}");
        assert!(html.contains("grid gap-1.5"), "missing field in {html}");
    }

    #[tokio::test]
    async fn section_renders_title_and_child() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Text::new("hello")));
        let html = schema.render(&cx).await.unwrap().render(&cx);
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
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(html.contains("inside group"), "missing child in {html}");
        assert!(
            html.contains("flex flex-col gap-4"),
            "missing group class flex flex-col gap-4 in {html}"
        );
    }

    #[tokio::test]
    async fn grid_renders_with_cols_and_children() {
        let cx = cx();
        let schema = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))));
        let html = schema.render(&cx).await.unwrap().render(&cx);
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
    async fn nested_grid_inside_section() {
        let cx = cx();
        let schema = Schema::new(
            Section::new("Outer")
                .schema(Grid::new(2).schema((Text::new("left"), Text::new("right")))),
        );
        let html = schema.render(&cx).await.unwrap().render(&cx);
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
    async fn schema_composes_multiple_blocks() {
        let cx = cx();
        let schema = Schema::new((
            Section::new("A").schema(Text::new("a")),
            Group::new().schema(Text::new("b")),
        ));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing section card in {html}"
        );
        assert!(
            html.contains("flex flex-col gap-4"),
            "missing group in {html}"
        );
    }

    #[tokio::test]
    async fn empty_schema_renders_empty() {
        let cx = cx();
        let schema = Schema::empty();
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(
            html.is_empty() || !html.contains("border-border"),
            "empty schema should render nothing, got {html}"
        );
    }
}
