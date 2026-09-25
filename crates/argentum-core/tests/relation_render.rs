//! The read-only relation table (GH #187 item 6, GH #296).

use argentum_core::{
    MAX_RELATION_ROWS, RelationColumn, RelationColumns, Resource, render_relation,
};
use topcoat::{
    context::{Cx, CxTestBuilder},
    view::ViewExt,
};

#[derive(Debug, Clone, toasty::Model)]
struct Row {
    #[key]
    #[auto]
    id: uuid::Uuid,
    name: String,
}

fn row(name: &str) -> Row {
    Row {
        id: uuid::Uuid::new_v4(),
        name: name.to_string(),
    }
}

fn columns() -> RelationColumns<Row> {
    RelationColumns::columns((
        RelationColumn::computed("Name", |r: &Row| r.name.clone()),
        RelationColumn::computed("Shout", |r: &Row| r.name.to_uppercase()),
    ))
}

fn rows() -> Vec<Row> {
    vec![row("first"), row("second")]
}

/// The related resource of a table that shows every row.
struct AllRows;

impl Resource for AllRows {
    type Model = Row;

    fn can_view(_cx: &Cx, _record: &Row) -> bool {
        true
    }
}

/// The related resource of a table that refuses the rows named `denied`.
struct NamedRows;

impl Resource for NamedRows {
    type Model = Row;

    fn can_view(_cx: &Cx, record: &Row) -> bool {
        record.name != "denied-row"
    }
}

#[tokio::test]
async fn a_relation_table_renders_every_row_and_column() {
    let cx = CxTestBuilder::new().build();
    let html = render_relation::<AllRows>(&cx, "Related", columns(), &rows())
        .single()
        .await
        .unwrap()
        .render(&cx);
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
    let rows = vec![row("<script>alert(1)</script>")];
    let html = render_relation::<AllRows>(&cx, "Related", columns(), &rows)
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
    let html = render_relation::<AllRows>(&cx, "Related", columns(), &[])
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

/// The related resource's `can_view` decides which rows render (GH #296).
///
/// The table is the surface that reads a fixed set of loaded rows, so the
/// predicate runs here rather than in the caller's projection: a row it refuses
/// must not reach a column's `display` closure.
#[tokio::test]
async fn a_relation_omits_rows_the_related_resource_refuses() {
    let cx = CxTestBuilder::new().build();
    let rows = vec![row("kept-a"), row("denied-row"), row("kept-b")];
    let html = render_relation::<NamedRows>(&cx, "Related", columns(), &rows)
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        !html.contains("denied-row"),
        "a refused row must not render: {html}"
    );
    for kept in ["kept-a", "kept-b"] {
        assert!(html.contains(kept), "an admitted row renders: {html}");
    }
}

/// A relation caps its rows and names the truncation (GH #296).
///
/// The related rows are already loaded, so the cap bounds the page rather than
/// a query; a table that stopped at the cap without a line saying so would read
/// as the whole relation.
#[tokio::test]
async fn a_relation_caps_its_rows_and_says_so() {
    let cx = CxTestBuilder::new().build();
    let total = MAX_RELATION_ROWS + 3;
    let rows: Vec<Row> = (0..total).map(|i| row(&format!("r{i:03}"))).collect();
    let html = render_relation::<AllRows>(&cx, "Related", columns(), &rows)
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        html.contains(&format!("r{:03}", MAX_RELATION_ROWS - 1)),
        "the table renders up to the cap: {html}"
    );
    assert!(
        !html.contains(&format!("r{MAX_RELATION_ROWS:03}")),
        "the table renders no row past the cap: {html}"
    );
    assert!(
        html.contains(&format!(
            "the first {MAX_RELATION_ROWS} of {total} related rows"
        )),
        "a truncated table names the cap and the total: {html}"
    );
}
