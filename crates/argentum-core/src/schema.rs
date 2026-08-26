//! Unified Schema primitive — layout blocks that compose via `view!`.
//!
//! `Schema` is a container for `Section`, `Group`, `Grid` and `Text` nodes.
//! Each node renders through Topcoat's `view!` macro; `Schema::render`
//! combines them. The API mirrors Filament's `Schema::new(( ... ))` tuple
//! form via the `IntoSchema` trait.

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
        view! { cx => <div class="ac-text">(content)</div> }
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
    pub fn for_lens<M, T>(path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
    {
        let (field_name, label) = lens_field_name_and_label(path);
        Self {
            name: field_name,
            label,
            required: false,
            is_email: false,
            placeholder: None,
        }
    }

    /// Convenience alias so call sites read `TextInput::for(User::fields().name())`.
    pub fn r#for<M, T>(path: toasty::stmt::Path<M, T>) -> Self
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
        // TODO: slice 2 — use `validator` crate for email
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
        let label = self.label.clone();
        let name = self.name.clone();
        let required = self.required;
        let placeholder = self.placeholder.clone();
        let input_type = if self.is_email { "email" } else { "text" };
        // Single template — placeholder omitted when None, `required` attr set,
        // label linked via `for`/`id`, star aria-hidden.
        // Review P3: inline `validate()` errors are not rendered here — form
        // state + per-field error slots belong with Schema form state and
        // #[procedure] handling (Slice 4). Static demo in showcase/schema.rs
        // shows errors outside ac-field; wiring them inside the field is deferred.
        view! { cx => <div class="ac-field"><label class="ac-field-label" for=(name.clone())>(label) if required { <span class="ac-required" aria-hidden="true">"*"</span> } </label><input id=(name.clone()) type=(input_type) name=(name) placeholder=(placeholder) required=(required) class="ac-input" /></div> }
    }
}

/// Spec alias — ADR-0001 typed lens. Slice 1 uses `toasty::stmt::Path` directly as the lens;
/// a richer `FieldLens` trait (carrying `FieldTy`, nullability, etc.) will replace this alias in slice 2.
///
/// Review P8: `is_nullable` / `is_unique` / `column_name` are not needed until
/// Create/Edit hydration (hydration → Create/Update projections). Deferred to
/// Slice 4 and tracked as tech debt — do not add now.
pub type FieldLens<M, T> = toasty::stmt::Path<M, T>;

/// Resolve a typed lens to its app-level field name and capitalized label.
///
/// Hides the `Path → toasty_core::stmt::Path → projection → M::schema()` walk
/// (review S1/S4). Used by both `TextInput` and `TextColumn` so the shape is
/// defined once.
pub(crate) fn lens_field_name_and_label<M, T>(path: FieldLens<M, T>) -> (String, String)
where
    M: toasty::schema::Model,
{
    let core_path: toasty_core::stmt::Path = path.into();
    debug_assert!(
        !core_path.projection.as_slice().is_empty(),
        "lens expects a field lens, got root path"
    );
    // Slice 1: only single-field lenses; multi-step paths will panic in
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
    let label = capitalize(&field_name);
    (field_name, label)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// PK tie-breaker(s) for deterministic pagination (review P5 core coupling).
///
/// Centralizes the `toasty_core::stmt` walk (`M::schema().as_root().primary_key`)
/// so `resource.rs` does not directly depend on core internals. Returns one
/// `asc` `OrderByExpr` per PK field, in declared order.
pub(crate) fn pk_tie_breakers<M>() -> Vec<toasty::stmt::OrderByExpr>
where
    M: toasty::schema::Model,
{
    let mut out = Vec::new();
    let app_model = M::schema();
    if let Some(root) = app_model.as_root() {
        for pk_field in &root.primary_key.fields {
            let expr = toasty_core::stmt::Expr::ref_self_field(*pk_field);
            out.push(toasty::stmt::OrderByExpr {
                expr,
                order: Some(toasty_core::stmt::Direction::Asc),
            });
        }
    }
    out
}

/// Section — titled container with an optional child `Schema`.
#[derive(Debug)]
pub struct Section {
    title: String,
    children: Option<Schema>,
}

impl Section {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            children: None,
        }
    }

    pub fn schema(mut self, children: impl IntoSchema) -> Self {
        self.children = Some(children.into_schema());
        self
    }

    async fn render(&self, cx: &Cx) -> Result<View> {
        let title = self.title.clone();
        if let Some(schema) = &self.children {
            let child_view = schema.render(cx).await?;
            view! {
                cx =>
                <section class="ac-section">
                    <h2 class="ac-section-title">(title)</h2>
                    <div class="ac-section-content">(child_view)</div>
                </section>
            }
        } else {
            view! {
                cx =>
                <section class="ac-section">
                    <h2 class="ac-section-title">(title)</h2>
                </section>
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
            view! { cx => <div class="ac-group">(child_view)</div> }
        } else {
            view! { cx => <div class="ac-group"></div> }
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
        let class = format!("ac-grid ac-grid-cols-{}", self.cols);
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

// Slice 1: one field variant (TextInput). Will generalize to `Field` (enum of all field types) in slice 2 per spec `Node::Field`.
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
// 4-tuple limit is intentional (review S2): without variadic generics this is
// idiomatic — see `IntoColumns` in `resource.rs`. Macro deferred until 5+
// columns are needed.
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

    // ---- Slice 1: TextInput with typed lens (red) ----

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
        assert!(html.contains("ac-field"), "missing ac-field in {html}");
        assert!(html.contains("ac-input"), "missing ac-input in {html}");
        assert!(
            html.contains("name=\"name\"")
                || html.contains("name=\"Name\"")
                || html.contains("name"),
            "missing name attr in {html}"
        );
        assert!(html.contains("<input"), "missing input in {html}");
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
            input.validate("   ").is_empty() == false,
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
            html_req.contains("ac-required"),
            "required should render star in {html_req}"
        );
        assert!(
            html_req.contains("required"),
            "required attr missing in {html_req}"
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
            html.matches("ac-field").count() >= 2,
            "expected 2 fields in {html}"
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
        assert!(html.contains("ac-section"), "missing section in {html}");
        assert!(html.contains("ac-grid"), "missing grid in {html}");
        assert!(html.contains("ac-field"), "missing field in {html}");
    }

    #[tokio::test]
    async fn section_renders_title_and_child() {
        let cx = cx();
        let schema = Schema::new(Section::new("Account").schema(Text::new("hello")));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(html.contains("Account"), "missing title in {html}");
        assert!(html.contains("hello"), "missing child in {html}");
        assert!(
            html.contains("ac-section"),
            "missing section class in {html}"
        );
        assert!(
            html.contains("ac-section-title"),
            "missing title class in {html}"
        );
    }

    #[tokio::test]
    async fn group_renders_children() {
        let cx = cx();
        let schema = Schema::new(Group::new().schema(Text::new("inside group")));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(html.contains("inside group"), "missing child in {html}");
        assert!(html.contains("ac-group"), "missing group class in {html}");
    }

    #[tokio::test]
    async fn grid_renders_with_cols_and_children() {
        let cx = cx();
        let schema = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(html.contains("ac-grid"), "missing grid class in {html}");
        assert!(
            html.contains("ac-grid-cols-2"),
            "missing cols class in {html}"
        );
        assert!(
            html.contains(">a<") || html.contains("ac-text\">a"),
            "missing a in {html}"
        );
        assert!(
            html.contains(">b<") || html.contains("ac-text\">b"),
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
        assert!(html.contains("ac-section"), "missing section in {html}");
        assert!(html.contains("ac-grid"), "missing grid in {html}");
    }

    #[tokio::test]
    async fn schema_composes_multiple_blocks() {
        let cx = cx();
        let schema = Schema::new((
            Section::new("A").schema(Text::new("a")),
            Group::new().schema(Text::new("b")),
        ));
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(html.contains("ac-section"), "missing section in {html}");
        assert!(html.contains("ac-group"), "missing group in {html}");
    }

    #[tokio::test]
    async fn empty_schema_renders_empty() {
        let cx = cx();
        let schema = Schema::empty();
        let html = schema.render(&cx).await.unwrap().render(&cx);
        assert!(
            html.is_empty() || !html.contains("ac-"),
            "empty schema should render nothing, got {html}"
        );
    }
}
