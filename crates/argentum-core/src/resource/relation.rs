//! Read-only relation rendering for detail pages (GH #187).
//!
//! A detail page shows a record's related rows — a post's comments — from the
//! rows `Resource::query`'s `include` already loaded. This renders them, and
//! deliberately renders *only* them: no pager, no search, no row actions, no
//! bulk column. A relation on a record page is a fixed, already-loaded set,
//! and the list's chrome exists to narrow a query this page never runs.
//!
//! Two bounds apply to that set (GH #296): the related resource's `can_view`
//! decides which rows the reader may see, and [`MAX_RELATION_ROWS`] caps how
//! many render. Both are in-memory decisions over the loaded rows, so the
//! relation still issues no query.
//!
//! Everything here is owned: a column's projection returns a `String`, so the
//! rendered view borrows the request context and nothing else. That is what
//! lets `Resource::view_relations(cx, record)` return a view that outlives the
//! record it read — the page is rendered before the handler's `record` binding
//! drops, and the borrow checker says so if it is not.

use argentum_ui::{table, table_body, table_cell, table_head, table_header, table_row};
use topcoat::{
    context::Cx,
    view::{BoxView, ViewExt, view},
};

use super::Resource;

/// The most rows a relation table renders (GH #296).
///
/// The cap bounds the rendered page, not a query: the related rows are already
/// loaded, so what it removes is cell projection and DOM size. 50 covers the
/// one-to-many sets a detail page summarises while keeping a runaway relation
/// from making the page unusable; the table prints an overflow line when it
/// truncates, so a capped relation never reads as a complete one.
pub const MAX_RELATION_ROWS: usize = 50;

/// One column of a relation's read-only table (GH #187).
///
/// The projection is the same shape a list column uses — a typed closure over
/// the related record — minus everything that only makes sense against a
/// query: no `sortable`, no `searchable`, no key.
pub struct RelationColumn<R> {
    label: String,
    display: Box<dyn Fn(&R) -> String + Send + Sync>,
}

impl<R> RelationColumn<R> {
    /// Declare a column by label and projection.
    ///
    /// Named for the shape it is, as
    /// [`TextColumn::computed`](crate::resource::TextColumn::computed) is on the list's side: a
    /// relation column has no lens to bind, because the related rows arrive as values rather
    /// than as a query. What a column *is* is its projection.
    pub fn computed(
        label: impl Into<String>,
        display: impl Fn(&R) -> String + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            display: Box::new(display),
        }
    }

    /// The column's heading.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// This column's cell for `row`.
    pub fn render_cell(&self, row: &R) -> String {
        (self.display)(row)
    }
}

impl<R> std::fmt::Debug for RelationColumn<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelationColumn")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// The columns of a relation's table, declared as a tuple or a single column.
///
/// A separate collection from the list's `IntoColumns` because the two tables
/// answer different questions: one is queryable, this one is not.
pub struct RelationColumns<R> {
    pub(crate) columns: Vec<RelationColumn<R>>,
}

impl<R> RelationColumns<R> {
    /// Collect one or more columns.
    pub fn columns(columns: impl IntoRelationColumns<R>) -> Self {
        columns.into_relation_columns()
    }
}

/// What can be collected into [`RelationColumns`].
pub trait IntoRelationColumns<R> {
    fn into_relation_columns(self) -> RelationColumns<R>;
}

impl<R> IntoRelationColumns<R> for RelationColumn<R> {
    fn into_relation_columns(self) -> RelationColumns<R> {
        RelationColumns {
            columns: vec![self],
        }
    }
}

impl<R, A, B> IntoRelationColumns<R> for (A, B)
where
    A: IntoRelationColumns<R>,
    B: IntoRelationColumns<R>,
{
    fn into_relation_columns(self) -> RelationColumns<R> {
        let mut columns = self.0.into_relation_columns().columns;
        columns.extend(self.1.into_relation_columns().columns);
        RelationColumns { columns }
    }
}

/// Render `rows` as a titled, read-only table (GH #187, GH #296).
///
/// `R` is the related resource. Its [`can_view`](Resource::can_view) is applied
/// to every row before that row is projected, so a relation declared through the
/// framework cannot render a row the reader may not see; naming the resource at
/// the call (`render_relation::<CommentResource>(cx, …)`) is what makes the
/// policy the resource's, rather than a filter the caller writes and a later
/// edit drops. Only the per-row predicate runs: `can_view_any` gates the related
/// resource's own list page, which this table does not render.
///
/// `rows` is the related records the record already carries — the caller passes
/// `record.comments.get().iter().cloned().collect()` or the equivalent — so this
/// runs no query, and `can_view` is an in-memory predicate over those rows for
/// the same reason. At most [`MAX_RELATION_ROWS`] rows render, with a line
/// naming the truncation below the table. An empty set renders the title with an
/// honest "none" line rather than an empty table, which would read as a failure
/// to load.
///
/// The rows are rendered in the order given; a detail page shows what the query
/// loaded, and `Resource::query` owns that order.
pub fn render_relation<'a, R: Resource>(
    cx: &'a Cx,
    title: &str,
    columns: RelationColumns<R::Model>,
    rows: &[R::Model],
) -> BoxView<'a> {
    let title = title.to_string();
    // Own everything before the `view!` block: the emitted view must borrow
    // the request context and nothing else, or the caller's `record` (which
    // owns these rows) would have to outlive the page.
    let heads: Vec<String> = columns
        .columns
        .iter()
        .map(|c| c.label().to_string())
        .collect();
    // Policy first, then the cap: a row the reader may not see is not a row
    // this table shows, and it must not consume a slot a visible row needs.
    let visible: Vec<&R::Model> = rows.iter().filter(|row| R::can_view(cx, row)).collect();
    let total = visible.len();
    let cells: Vec<Vec<String>> = visible
        .iter()
        .take(MAX_RELATION_ROWS)
        .map(|row| {
            columns
                .columns
                .iter()
                .map(|column| column.render_cell(row))
                .collect()
        })
        .collect();
    let has_rows = !cells.is_empty();
    let overflow = (total > MAX_RELATION_ROWS).then(|| {
        format!("Showing the first {MAX_RELATION_ROWS} of {total} related rows you can view.")
    });
    // Row ids are positional: this table does not reorder or swap, so it needs
    // no record key — the list's `Table::id` contract exists for keyed diffs
    // and action URLs, and neither exists here.
    view! {
        cx =>
        <section class="flex flex-col gap-3">
            <h2 class="text-base font-semibold">(title)</h2>
            if !has_rows {
                <p class="text-sm text-muted-foreground">"None."</p>
            } else {
                table(
                    table_header(
                        table_row(
                            for head in heads {
                                table_head((head))
                            }
                        )
                    )
                    table_body(
                        for row in cells {
                            table_row(
                                for cell in row {
                                    table_cell((cell))
                                }
                            )
                        }
                    )
                )
                if let Some(overflow) = overflow {
                    <p class="text-sm text-muted-foreground">(overflow)</p>
                }
            }
        </section>
    }
    .boxed()
}
