//! Table columns: [`TextColumn`]/[`Column`] plus the [`IntoColumns`] seam.
//!
//! Moved verbatim from `resource.rs` (GH #133): no behavior change.

use std::sync::Arc;

use toasty::stmt::{Expr, OrderByExpr};

use crate::schema::{FieldLens, lens_field, lens_label};

/// Text column bound to a typed lens **and** a typed projection.
///
/// The lens (`FieldLens<M, String>`) is the query side: it names the column
/// and produces search/sort predicates — `TextColumn::for(User::fields().name(), ..)`
/// fails to compile if the field does not exist (ADR-0001).
///
/// The projection closure is the render side: it reads the value off a model
/// instance for the cell (`|u| u.name.clone()`). Toasty models are plain
/// structs and expose no instance→field reflection, so the closure is the
/// only way to read a field generically (upstream gap #119: instance →
/// field-value extraction). A typo in the closure body fails at compile
/// time — there is no string dispatch and no panic at render.
#[derive(Clone)]
pub struct TextColumn<M> {
    /// The query-side lens; `None` for [`Self::computed`] columns, which
    /// render a value but declare no predicates.
    path: Option<FieldLens<M, String>>,
    name: String,
    label: String,
    project: Arc<dyn Fn(&M) -> String + Send + Sync>,
    searchable: bool,
    sortable: bool,
}

/// The escape character the search pattern declares to `LIKE` (GH #116):
/// backslash, escaped in the pattern by [`escape_like_pattern`].
pub(crate) const LIKE_ESCAPE: char = '\\';

/// Wrap `term` as a `LIKE` pattern matching it anywhere in the column, with
/// `%`, `_` and the escape character itself escaped so the term stays literal
/// (GH #116).
///
/// Toasty ships the SQL half (`like_with_escape`) but not this one: escaping is
/// app-side because only the app knows whether it is building a literal or a
/// pattern.
pub(crate) fn escape_like_pattern(term: &str) -> String {
    let mut pattern = String::with_capacity(term.len() + 2);
    pattern.push('%');
    for c in term.chars() {
        if c == LIKE_ESCAPE || c == '%' || c == '_' {
            pattern.push(LIKE_ESCAPE);
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

impl<M> TextColumn<M>
where
    M: toasty::schema::Model,
{
    /// Bind a column to a `String` field lens plus a projection closure:
    /// `TextColumn::for(User::fields().name(), |u| u.name.clone())`.
    ///
    /// The closure receives each rendered row and returns the cell text, so
    /// computed cells (`|u| u.active.then(|| "Active".into()).unwrap_or_default()`)
    /// are as natural as plain field reads.
    pub fn r#for(
        path: FieldLens<M, String>,
        project: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        let field = lens_field(path.clone(), &M::schema());
        Self {
            path: Some(path),
            name: field.name.app_unwrap().to_string(),
            label: lens_label(&field),
            project: Arc::new(project),
            searchable: false,
            sortable: false,
        }
    }

    /// A computed, display-only column (CONTEXT.md Column: "a computed value").
    ///
    /// No field lens — so it cannot be searchable or sortable (it maps to no
    /// query predicate) — but any cell projection compiles: booleans,
    /// timestamps, joined values. Calling `.searchable()` / `.sortable()` on
    /// a computed column panics (GH #101): a lying sort link / search promise
    /// is worse than a loud build error.
    pub fn computed(
        label: impl Into<String>,
        project: impl Fn(&M) -> String + Send + Sync + 'static,
    ) -> Self {
        let label = label.into();
        let name = label.to_lowercase();
        Self {
            path: None,
            name,
            label,
            project: Arc::new(project),
            searchable: false,
            sortable: false,
        }
    }

    pub fn searchable(mut self) -> Self {
        assert!(
            self.path.is_some(),
            "searchable() on computed column '{}': computed columns map to no query predicate",
            self.label
        );
        self.searchable = true;
        self
    }

    pub fn sortable(mut self) -> Self {
        assert!(
            self.path.is_some(),
            "sortable() on computed column '{}': computed columns map to no query predicate",
            self.label
        );
        self.sortable = true;
        self
    }

    pub fn is_searchable(&self) -> bool {
        self.searchable
    }

    pub fn is_sortable(&self) -> bool {
        self.sortable
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// App-level field name (from the lens). Identifies the column in the
    /// `?sort=` URL parameter.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Render the cell for one row via the typed projection.
    pub fn render_cell(&self, row: &M) -> String {
        (self.project)(row)
    }

    /// The search predicate for this column (GH #116): a portable, escaped
    /// **substring** match.
    ///
    /// `like_with_escape` keeps the pattern parameterised and lowers to the
    /// same `LIKE … ESCAPE '\\'` on every driver, and
    /// [`escape_like_pattern`] makes the term literal — a `%` or `_` the user
    /// typed matches that character, it does not act as a wildcard. Note the
    /// driver difference `LIKE` brings: SQLite compares ASCII
    /// case-insensitively, PostgreSQL case-sensitively.
    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        let t = term.trim();
        if !self.searchable || t.is_empty() {
            return None;
        }
        Some(
            self.path
                .clone()?
                .like_with_escape(escape_like_pattern(t), LIKE_ESCAPE),
        )
    }

    pub fn to_order_by(&self, descending: bool) -> Option<OrderByExpr> {
        if self.sortable {
            // Cursor determinism is the engine's job: toasty's
            // `normalize_cursor_order` appends the physical PK columns to
            // ambiguous cursor orderings internally (GH #76).
            let path = self.path.clone()?;
            Some(if descending { path.desc() } else { path.asc() })
        } else {
            None
        }
    }
}

impl<M> std::fmt::Debug for TextColumn<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextColumn")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("searchable", &self.searchable)
            .field("sortable", &self.sortable)
            .finish_non_exhaustive()
    }
}

/// Column enum — Phase 1: only `Text`. Will generalize to `Number`, `Badge`, etc. later.
#[derive(Clone)]
pub enum Column<M> {
    Text(TextColumn<M>),
}

impl<M> From<TextColumn<M>> for Column<M> {
    fn from(v: TextColumn<M>) -> Self {
        Column::Text(v)
    }
}

impl<M> Column<M>
where
    M: toasty::schema::Model,
{
    pub fn label(&self) -> &str {
        match self {
            Column::Text(c) => c.label(),
        }
    }

    /// App-level field name (from the lens); identifies the column in URLs.
    pub fn name(&self) -> &str {
        match self {
            Column::Text(c) => c.name(),
        }
    }

    pub fn is_searchable(&self) -> bool {
        match self {
            Column::Text(c) => c.is_searchable(),
        }
    }

    pub fn is_sortable(&self) -> bool {
        match self {
            Column::Text(c) => c.is_sortable(),
        }
    }

    /// Render the cell for one row via the column's typed projection.
    pub fn render_cell(&self, row: &M) -> String {
        match self {
            Column::Text(c) => c.render_cell(row),
        }
    }

    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        match self {
            Column::Text(c) => c.to_search_expr(term),
        }
    }

    pub fn to_order_by(&self, descending: bool) -> Option<OrderByExpr> {
        match self {
            Column::Text(c) => c.to_order_by(descending),
        }
    }
}

/// Convert a single column or tuple of columns into `Vec<Column<M>>`.
///
/// 5-tuple limit: without variadic generics this is idiomatic Rust — matches
/// `IntoSchema` in `schema.rs`. Tables wider than five columns are rare in
/// admin UIs; extend (or macro-ify) when a real Resource needs it.
pub trait IntoColumns<M> {
    fn into_columns(self) -> Vec<Column<M>>;
}

impl<M> IntoColumns<M> for TextColumn<M> {
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.into()]
    }
}

impl<M> IntoColumns<M> for Column<M> {
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self]
    }
}

impl<M, A, B> IntoColumns<M> for (A, B)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into()]
    }
}

impl<M, A, B, C> IntoColumns<M> for (A, B, C)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into(), self.2.into()]
    }
}

impl<M, A, B, C, D> IntoColumns<M> for (A, B, C, D)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
    D: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![self.0.into(), self.1.into(), self.2.into(), self.3.into()]
    }
}

impl<M, A, B, C, D, E> IntoColumns<M> for (A, B, C, D, E)
where
    A: Into<Column<M>>,
    B: Into<Column<M>>,
    C: Into<Column<M>>,
    D: Into<Column<M>>,
    E: Into<Column<M>>,
{
    fn into_columns(self) -> Vec<Column<M>> {
        vec![
            self.0.into(),
            self.1.into(),
            self.2.into(),
            self.3.into(),
            self.4.into(),
        ]
    }
}

impl<M> std::fmt::Debug for Column<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Column::Text(c) => std::fmt::Debug::fmt(c, f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    /// GH #116: `%` and `_` in a search term are literal characters, not
    /// wildcards, and the term is wrapped for a substring match.
    #[test]
    fn search_pattern_escapes_like_metacharacters() {
        assert_eq!(escape_like_pattern("Ada"), "%Ada%");
        assert_eq!(escape_like_pattern("100%"), "%100\\%%");
        assert_eq!(escape_like_pattern("a_b"), "%a\\_b%");
        assert_eq!(escape_like_pattern("back\\slash"), "%back\\\\slash%");
    }

    #[test]
    fn text_column_searchable_produces_a_substring_pattern() {
        let col = TextColumn::r#for(User::fields().name(), |u| u.name.clone()).searchable();
        assert!(
            col.to_search_expr("Ada").is_some(),
            "searchable should produce expr"
        );
        assert!(
            TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                .to_search_expr("Ada")
                .is_none(),
            "non-searchable should be None"
        );
    }

    #[test]
    fn text_column_sortable_produces_order_by() {
        let col = TextColumn::r#for(User::fields().name(), |u| u.name.clone()).sortable();
        assert!(
            col.to_order_by(false).is_some(),
            "sortable should produce order_by"
        );
        assert!(
            TextColumn::r#for(User::fields().name(), |u| u.name.clone())
                .to_order_by(false)
                .is_none(),
            "non-sortable should be None"
        );
    }

    #[test]
    fn text_column_renders_cells_via_typed_projection() {
        let plain = TextColumn::r#for(User::fields().name(), |u| u.name.clone());
        let decorated = TextColumn::r#for(User::fields().name(), |u| format!("{}!", u.name));
        let row = User {
            id: uuid::Uuid::nil(),
            name: "Ada".to_string(),
        };
        assert_eq!(plain.render_cell(&row), "Ada");
        assert_eq!(decorated.render_cell(&row), "Ada!");
        assert_eq!(plain.name(), "name");
        assert_eq!(plain.label(), "Name");
    }

    #[test]
    #[should_panic(expected = "searchable() on computed column")]
    fn computed_searchable_panics_loudly() {
        let _ = TextColumn::computed("Status", |u: &User| u.name.clone()).searchable();
    }

    #[test]
    #[should_panic(expected = "sortable() on computed column")]
    fn computed_sortable_panics_loudly() {
        let _ = TextColumn::computed("Status", |u: &User| u.name.clone()).sortable();
    }

    #[test]
    fn computed_columns_declare_no_predicate_chrome_agreement() {
        let col = TextColumn::computed("Status", |u: &User| u.name.clone());
        assert!(!col.is_searchable() && !col.is_sortable());
        assert!(col.to_search_expr("x").is_none());
        assert!(col.to_order_by(false).is_none());
    }
}
