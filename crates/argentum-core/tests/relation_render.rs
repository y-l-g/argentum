//! The read-only relation table (GH #187 item 6).

use argentum_core::{RelationColumn, RelationColumns, render_relation};
use topcoat::{context::CxTestBuilder, view::ViewExt};

#[derive(Debug, Clone)]
struct Row {
    name: String,
}

fn columns() -> RelationColumns<Row> {
    RelationColumns::columns((
        RelationColumn::computed("Name", |r: &Row| r.name.clone()),
        RelationColumn::computed("Shout", |r: &Row| r.name.to_uppercase()),
    ))
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            name: "first".to_string(),
        },
        Row {
            name: "second".to_string(),
        },
    ]
}

#[tokio::test]
async fn a_relation_table_renders_every_row_and_column() {
    let cx = CxTestBuilder::new().build();
    let html = render_relation(&cx, "Related", columns(), &rows())
        .single()
        .await
        .unwrap()
        .render(&cx);
    eprintln!("RENDERED: {html}");
    for expected in [
        "Related", "Name", "Shout", "first", "FIRST", "second", "SECOND",
    ] {
        assert!(html.contains(expected), "missing {expected:?} in {html}");
    }
}

/// A cell is text, not markup (GH #187).
#[tokio::test]
async fn a_relation_cell_is_escaped() {
    let cx = CxTestBuilder::new().build();
    let rows = vec![Row {
        name: "<script>alert(1)</script>".to_string(),
    }];
    let html = render_relation(&cx, "Related", columns(), &rows)
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        !html.contains("<script>"),
        "a stored value must not become markup: {html}"
    );
    assert!(
        html.contains("&lt;script&gt;"),
        "the value renders escaped instead: {html}"
    );
}

#[tokio::test]
async fn an_empty_relation_says_none() {
    let cx = CxTestBuilder::new().build();
    let html = render_relation(&cx, "Related", columns(), &[])
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(html.contains("None."), "empty relation: {html}");
    assert!(
        !html.contains("<table"),
        "an empty relation renders no table: {html}"
    );
}
