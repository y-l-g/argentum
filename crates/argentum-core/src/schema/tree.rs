//! Schema tree — `Node`, `IntoSchema`, and the tree walks.
//!
//! `Node` composes field leaves (`fields`) and containers (`layouts`)
//! into one tree; `for_each_field` is the single traversal the facade
//! collectors share, and `walk_repeater_absence` classifies repeaters.

use std::collections::{HashMap, HashSet};

use topcoat::runtime::Signal;
use topcoat::{Result, context::Cx, view::*};

use super::Schema;
use super::fields::{FileUpload, Select, Text, TextInput};
use super::layouts::{Grid, Group, Repeater, Section, Tabs, Wizard};

#[derive(Debug)]
pub(crate) enum Node {
    Text(Text),
    TextInput(Box<TextInput>),
    Select(Box<Select>),
    FileUpload(Box<FileUpload>),
    Repeater(Box<Repeater>),
    Tabs(Box<Tabs>),
    Wizard(Box<Wizard>),
    Section(Box<Section>),
    Group(Box<Group>),
    Grid(Box<Grid>),
}

impl Node {
    /// Render this node from `source` (internal; see
    /// [`Schema::render_with`] / [`Schema::render_live_with`]).
    pub(crate) async fn render_source<'a>(
        &self,
        cx: &'a Cx,
        source: &RenderSource<'_>,
    ) -> Result<BoxView<'a>> {
        // A live field renders with its signal when the caller supplied a
        // value signal for it; the error list may be empty (a valid field).
        if let RenderSource::Live { values, errors } = source
            && let Node::TextInput(f) = self
            && let Some(value) = values.get(f.field_name())
        {
            let errs = errors
                .get(f.field_name())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            return Ok(Box::pin(f.render_live_with(cx, value, errs)).await?.boxed());
        }
        match self {
            Node::Text(t) => Ok(t.render(cx).await?.boxed()),
            Node::TextInput(f) => {
                let val = static_value(source, f.field_name());
                let errs = static_errors(source, f.field_name());
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::Select(f) => {
                let val = static_value(source, f.field_name());
                let errs = static_errors(source, f.field_name());
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::FileUpload(f) => {
                let val = static_value(source, f.field_name());
                let errs = static_errors(source, f.field_name());
                Ok(Box::pin(f.render_with(cx, val, errs)).await?.boxed())
            }
            Node::Repeater(r) => Ok(Box::pin(r.render_source(cx, source)).await?.boxed()),
            Node::Tabs(t) => Ok(Box::pin(t.render_source(cx, source)).await?.boxed()),
            Node::Wizard(w) => Ok(Box::pin(w.render_source(cx, source)).await?.boxed()),
            Node::Section(s) => Ok(Box::pin(s.render_source(cx, source)).await?.boxed()),
            Node::Group(g) => Ok(Box::pin(g.render_source(cx, source)).await?.boxed()),
            Node::Grid(g) => Ok(Box::pin(g.render_source(cx, source)).await?.boxed()),
        }
    }
}

/// Where a schema render reads field values and errors from (GH #154 §4).
///
/// `Static` is the create/edit path: plain maps, no bindings. `Live` carries
/// per-field value signals for form controls that must two-way bind.
pub(crate) enum RenderSource<'a> {
    Static {
        values: &'a HashMap<String, String>,
        errors: &'a HashMap<String, Vec<String>>,
    },
    Live {
        values: &'a HashMap<String, Signal<String>>,
        errors: &'a HashMap<String, Vec<String>>,
    },
}

/// The static value for `name`, if this render has one.
fn static_value<'a>(source: &'a RenderSource<'_>, name: &str) -> Option<&'a str> {
    match source {
        RenderSource::Static { values, .. } => values.get(name).map(String::as_str),
        RenderSource::Live { .. } => None,
    }
}

/// The static errors for `name`, if this render has any.
fn static_errors<'a>(source: &'a RenderSource<'_>, name: &str) -> &'a [String] {
    match source {
        RenderSource::Static { errors, .. } | RenderSource::Live { errors, .. } => {
            errors.get(name).map(|v| v.as_slice()).unwrap_or(&[])
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
impl From<Select> for Node {
    fn from(v: Select) -> Self {
        Node::Select(Box::new(v))
    }
}
impl From<FileUpload> for Node {
    fn from(v: FileUpload) -> Self {
        Node::FileUpload(Box::new(v))
    }
}
impl From<Repeater> for Node {
    fn from(v: Repeater) -> Self {
        Node::Repeater(Box::new(v))
    }
}
impl From<Tabs> for Node {
    fn from(v: Tabs) -> Self {
        Node::Tabs(Box::new(v))
    }
}
impl From<Wizard> for Node {
    fn from(v: Wizard) -> Self {
        Node::Wizard(Box::new(v))
    }
}

impl Node {
    /// Nested schema for container nodes; `None` for leaf fields.
    pub(crate) fn children(&self) -> Option<&Schema> {
        match self {
            Node::Repeater(r) => r.children.as_ref(),
            Node::Section(s) => s.children.as_ref(),
            Node::Group(g) => g.children.as_ref(),
            Node::Grid(g) => g.children.as_ref(),
            Node::Tabs(t) => t.0.children.as_ref(),
            Node::Wizard(w) => w.0.children.as_ref(),
            Node::TextInput(_) | Node::Select(_) | Node::FileUpload(_) | Node::Text(_) => None,
        }
    }
}

/// The one field walk: visit `node`, then every node nested in containers.
///
/// Container recursion lives here (with `Node::children`), so the
/// `*_inputs` collectors and `Schema::field_names` share one traversal: a
/// new field variant adds arms at their leaf matches, and a new container
/// variant touches only `children` plus this walk.
pub(crate) fn for_each_field(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    if let Some(children) = node.children() {
        for nested in &children.nodes {
            for_each_field(nested, f);
        }
    }
}

/// Classify the schema's repeaters against `values` (GH #147, replacing the
/// GH #75 required-walk and the optional-group skip in one tree walk).
///
/// For every Repeater, all its inner field names (as `field_names()` of the
/// child schema) are checked: an all-empty group is "absent" — an untouched
/// group submits empty strings or omits the keys, and both count as absent —
/// so its inner field names go into `skip` (their per-field `required` must
/// not fire, whatever the group's own requiredness) and its subtree is
/// walked as absent, suppressing nested required repeaters. A `required`
/// all-empty group also records its one label-keyed error, unless it sits
/// inside an already-absent ancestor. A group with any non-empty inner value
/// counts as present: nothing is skipped, nothing suppressed, and inner
/// `required` enforces as usual.
///
/// On edit, the GH #90 untouched-file backfill runs before validation, so a
/// group whose stored file path is non-empty counts as present there even if
/// the browser submitted it empty — a kept file is real group data.
pub(crate) fn walk_repeater_absence(
    nodes: &[Node],
    values: &HashMap<String, String>,
    skip: &mut HashSet<String>,
    errors: &mut HashMap<String, Vec<String>>,
    inside_absent: bool,
) {
    for node in nodes {
        if let Node::Repeater(r) = node {
            let inner_names = r
                .children
                .as_ref()
                .map(|s| s.field_names())
                .unwrap_or_default();
            // `all` on an empty list is true: an inputless group is absent.
            let all_empty = inner_names
                .iter()
                .all(|n| values.get(n).map(|v| v.trim().is_empty()).unwrap_or(true));
            let absent = inside_absent || all_empty;
            if all_empty {
                skip.extend(inner_names);
                if r.required && !inside_absent {
                    errors
                        .entry(r.label.clone())
                        .or_insert_with(|| vec![format!("{} is required", r.label)]);
                }
            }
            if let Some(child) = node.children() {
                walk_repeater_absence(&child.nodes, values, skip, errors, absent);
            }
            continue;
        }
        if let Some(child) = node.children() {
            walk_repeater_absence(&child.nodes, values, skip, errors, inside_absent);
        }
    }
}

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
impl IntoSchema for Select {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for FileUpload {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Repeater {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Tabs {
    fn into_schema(self) -> Schema {
        Schema {
            nodes: vec![self.into()],
        }
    }
}
impl IntoSchema for Wizard {
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

#[cfg(test)]
mod tests {
    use topcoat::context::{Cx, CxTestBuilder};

    use crate::schema::{Group, Schema, Section, Text, TextInput};

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
    async fn text_input_composes_in_tuple() {
        let cx = cx();
        let schema = Schema::new((
            TextInput::r#for(DummyUser::fields().name()),
            TextInput::r#for(DummyUser::fields().email()),
        ));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.matches("data-slot=\"field\"").count() >= 2,
            "expected 2 fields (data-slot=field) in {html}"
        );
        assert!(
            html.matches("text-sm text-destructive").count() >= 2,
            "expected 2 error slots in {html}"
        );
    }

    #[tokio::test]
    async fn schema_composes_multiple_blocks() {
        let cx = cx();
        let schema = Schema::new((
            Section::new("A").schema(Text::new("a")),
            Group::new().schema(Text::new("b")),
        ));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("rounded-xl") && html.contains("border-border"),
            "missing section card in {html}"
        );
        assert!(
            html.contains("@container/field-group"),
            "missing group in {html}"
        );
    }

    #[tokio::test]
    async fn empty_schema_renders_empty() {
        let cx = cx();
        let schema = Schema::empty();
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.trim().is_empty(),
            "empty schema should render nothing, got {html}"
        );
    }
}
