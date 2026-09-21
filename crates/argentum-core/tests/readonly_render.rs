//! The read-only side of a `Schema` (GH #187): what a detail page renders.
//!
//! These pin the *shape* — the promise that a view is a reading of a record,
//! not a disabled form — because the showcase's HTTP tests can only check that
//! the page answers and shows a value, and the shape is what a detail page
//! would get wrong quietly (an `<input>` in the markup is invisible to a test
//! that greps for the value).

use std::collections::HashMap;

use argentum_core::schema::{
    FileUpload, Grid, Group, Repeater, Schema, Section, Select, TextInput, Textarea,
};
use toasty::Db;
use topcoat::context::{Cx, CxTestBuilder};
use topcoat::view::ViewExt;

#[derive(Debug, toasty::Model)]
struct Doc {
    #[key]
    #[auto]
    id: uuid::Uuid,
    title: String,
    body: String,
    status: String,
    path: String,
}

async fn cx() -> Cx {
    let db = Db::builder()
        .models(toasty::models!(Doc))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    CxTestBuilder::new().app_context(db).build()
}

fn values() -> HashMap<String, String> {
    HashMap::from([
        ("title".to_string(), "A Title".to_string()),
        ("body".to_string(), "Line one\nLine two".to_string()),
        ("status".to_string(), "published".to_string()),
        ("path".to_string(), "/uploads/cover.jpg".to_string()),
    ])
}

/// The rendered page body, as one string.
async fn render(schema: &Schema, values: &HashMap<String, String>) -> String {
    let cx = cx().await;
    schema
        .render_readonly(&cx, values)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx)
}

#[tokio::test]
async fn a_view_renders_values_not_controls() {
    let schema = Schema::new((
        TextInput::r#for(Doc::fields().title()),
        Textarea::r#for(Doc::fields().body()),
        Select::r#for(Doc::fields().status())
            .options(vec!["draft".to_string(), "published".to_string()]),
        FileUpload::r#for(Doc::fields().path()),
    ));
    let html = render(&schema, &values()).await;

    assert!(
        !html.contains("<input") && !html.contains("<select") && !html.contains("<textarea"),
        "a view renders values, never controls: {html}"
    );
    assert!(
        !html.contains("required") && !html.contains("aria-invalid"),
        "a view has nothing to validate: {html}"
    );
    // Every declared field is present, with its label and its value.
    for (label, value) in [
        ("Title", "A Title"),
        ("Body", "Line one\nLine two"),
        ("Status", "published"),
        ("Path", "/uploads/cover.jpg"),
    ] {
        assert!(html.contains(label), "missing label {label} in {html}");
        assert!(html.contains(value), "missing value {value} in {html}");
    }
}

#[tokio::test]
async fn a_select_renders_its_option_label() {
    // A stored key reads as the label the form offered, so the page shows what
    // the user chose rather than the wire value behind it.
    let schema = Schema::new(
        Select::r#for(Doc::fields().status())
            .options_with_labels(vec![("published".to_string(), "Live".to_string())]),
    );
    let html = render(&schema, &values()).await;
    assert!(
        html.contains("Live"),
        "the option label is what a view shows: {html}"
    );
}

#[tokio::test]
async fn a_select_without_a_matching_option_shows_the_stored_value() {
    // A value the options do not cover (a stale row, a relationship key) still
    // renders: a detail page shows what is stored, and must not blank a field
    // because it cannot name it.
    let schema =
        Schema::new(Select::r#for(Doc::fields().status()).options(vec!["draft".to_string()]));
    let html = render(&schema, &values()).await;
    assert!(
        html.contains("published"),
        "an uncovered value renders as itself: {html}"
    );
}

/// A group is a layout, so a view renders its label over its children's
/// values — and none of a form's affordances. A required group used to emit
/// the `*` marker and `aria-invalid` on the detail page, because the repeater
/// renders through its own path rather than a field's `render_with` (GH #187).
#[tokio::test]
async fn a_repeater_renders_its_children_without_form_affordances() {
    let schema = Schema::new(
        Repeater::new("Tags")
            .required()
            .schema(TextInput::r#for(Doc::fields().status())),
    );
    let html = render(&schema, &values()).await;
    assert!(html.contains("Tags"), "the group label renders: {html}");
    assert!(
        html.contains("published"),
        "a child field renders its value inside the group: {html}"
    );
    assert!(
        !html.contains("text-destructive")
            && !html.contains("aria-invalid")
            && !html.contains("ac-field--error"),
        "a read-only group has no required marker and nothing to be invalid about: {html}"
    );
}

#[tokio::test]
async fn an_empty_value_renders_as_empty() {
    // The framework stores `""` rather than NULL (GH #89), so a stored record
    // cannot distinguish absent from empty — and the page must not imply it
    // can (no "(none)", no dash, no placeholder text).
    let schema = Schema::new(TextInput::r#for(Doc::fields().title()));
    let absent = render(&schema, &HashMap::new()).await;
    // The framework stores `""`, never NULL, so a *present* empty value and an
    // absent key are the same record state and must read the same (GH #89).
    // Compare the value node rather than the whole markup: topcoat renders
    // attributes in no guaranteed order (topcoat#122), so two identical
    // renderings differ in attribute order.
    let present_empty = render(
        &schema,
        &HashMap::from([("title".to_string(), String::new())]),
    )
    .await;
    let value = |html: &str| {
        html.split_once("whitespace-pre-wrap\">")
            .and_then(|(_, rest)| rest.split_once("</div>"))
            .map(|(value, _)| value.to_string())
            .expect("each render has the value node")
    };
    assert_eq!(
        value(&absent),
        value(&present_empty),
        "an absent key and a stored empty value are one rendering"
    );
    assert!(value(&absent).is_empty(), "both are empty: {absent}");
    let html = absent;
    assert!(html.contains("Title"), "the label still renders: {html}");
    assert!(
        !html.contains("(none)")
            && !html.contains("(unloaded)")
            && !html.contains("placeholder")
            && !html.contains("—"),
        "an empty value is empty, with no invented marker: {html}"
    );
}

#[tokio::test]
async fn layout_blocks_keep_their_structure_around_values() {
    let schema = Schema::new(Section::new("Content").schema((
        Group::new().schema(Grid::new(2).schema((
            TextInput::r#for(Doc::fields().title()),
            TextInput::r#for(Doc::fields().status()),
        ))),
        Textarea::r#for(Doc::fields().body()),
    )));
    let html = render(&schema, &values()).await;
    assert!(html.contains("Content"), "section title survives: {html}");
    assert!(
        html.contains("grid-cols-2"),
        "a grid still lays its values out: {html}"
    );
    assert!(
        !html.contains("<input"),
        "structure must not reintroduce a control: {html}"
    );
}
