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

#[derive(Debug)]
enum Node {
    Text(Text),
    Section(Box<Section>),
    Group(Box<Group>),
    Grid(Box<Grid>),
}

impl Node {
    async fn render(&self, cx: &Cx) -> Result<View> {
        match self {
            Node::Text(t) => t.render(cx).await,
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
