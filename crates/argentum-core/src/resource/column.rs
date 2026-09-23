//! Table columns: [`TextColumn`] plus the [`IntoColumns`] seam.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use toasty::stmt::{Expr, OrderByExpr};

use crate::schema::{FieldLens, lens_field, lens_label};

/// The relations a query must `include`, named in the resource's vocabulary
/// (GH #177).
///
/// A column's projection closure reads relations off the row (`|p| p.author
/// .get().name.clone()`), and Toasty has no instance→field reflection
/// (upstream #119), so the framework cannot see *which* relation a closure
/// touches. Each column therefore **declares** the includes its closure reads
/// ([`TextColumn::needs`]), a [`Table`](super::Table) gathers the declarations
/// of the columns it renders into one `IncludeNeeds`, and
/// [`Resource::export_query`](super::Resource::export_query) answers `wants`
/// per `include(..)` call.
///
/// The names are an opaque vocabulary shared between the declaring column and
/// the resource that maps them onto `include(..)` calls, because includes are
/// typed (`Include<Post, Author>`) and a type-erased column cannot name one.
/// Nothing else reads them: an unknown name is not an error, it just never
/// matches a branch.
#[derive(Clone, Debug, Default)]
pub struct IncludeNeeds {
    names: BTreeSet<&'static str>,
}

impl IncludeNeeds {
    /// Whether `name` was declared by a rendered column.
    ///
    /// This is the one question a resource's
    /// [`export_query`](super::Resource::export_query) asks, once per
    /// `include(..)` it could add.
    pub fn wants(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// Whether no column declared anything — the narrowed query needs no
    /// relation at all.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

impl FromIterator<&'static str> for IncludeNeeds {
    fn from_iter<T: IntoIterator<Item = &'static str>>(iter: T) -> Self {
        Self {
            names: iter.into_iter().collect(),
        }
    }
}

/// `IncludeNeeds::from(["author", "comments"])` — the whole set up front, which
/// is what a resource's `query` needs (its `export_query` gets the set handed
/// to it instead).
impl<const N: usize> From<[&'static str; N]> for IncludeNeeds {
    fn from(names: [&'static str; N]) -> Self {
        names.into_iter().collect()
    }
}

/// The share of the table a [`ColumnWidth::Narrow`] column claims, in whole
/// percent (GH #240).
pub(crate) const NARROW_DEFAULT_PERCENT: u8 = 10;

/// The width a [`TextColumn`] claims in the table's fixed layout (GH #240).
///
/// Widths are **shares of the table**, so what a table declares is a fraction
/// of its container rather than a length that can outgrow it: the columns that
/// declare none take what the declared ones leave. A length
/// ([`Rem`](Self::Rem)) is the exception — lengths do not shrink with the
/// table, and a table whose lengths exceed its width gives the columns that
/// declare none no space at all, header text included.
///
/// The renderer writes the width into the column's `th` and every row's `td`
/// as an inline `style` attribute — data, never a generated Tailwind class.
/// Tailwind generates only the class literals it finds in source, so a width
/// assembled at render (`w-[{n}%]`) would emit no CSS at all (ADR-0006); a
/// declared width is read by the layout directly.
///
/// A column's **kind** picks the default: `TextColumn::r#for` binds a `String`
/// field, so its cells hold the row's own text — a title, a name, a body — and
/// it defaults to [`Wide`](Self::Wide), taking a share of what the declared
/// columns leave; [`TextColumn::computed`] derives its cell (a status, a
/// boolean, a date, a count) and defaults to [`Narrow`](Self::Narrow), a share
/// of the table. [`TextColumn::width`] overrides either, which is the seam for
/// a column whose content disagrees with its kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnWidth {
    /// Take a share of whatever the declared columns leave: the column
    /// declares no width, and `table-fixed` splits the remainder between the
    /// wide columns instead of measuring the rows currently rendered.
    #[default]
    Wide,
    /// A share of the table for a status, boolean, date or count cell: the
    /// default for a [`TextColumn::computed`] column. The renderer resolves
    /// the share (10% nominally) against the table's other kind defaults.
    Narrow,
    /// An explicit length in whole rem: `Rem(14)` declares `14rem`. A length
    /// does not shrink with the table, so a table narrower than the lengths it
    /// declares gives the columns that declare none no space at all.
    Rem(u8),
    /// An explicit share of the table in whole percent: `Percent(30)`
    /// declares `30%`.
    Percent(u8),
}

impl ColumnWidth {
    /// The share of the table this column claims as a **kind default**, in
    /// whole percent, or `None` for a column that declares an explicit width
    /// or none at all.
    ///
    /// A nominal: the renderer scales the kind defaults down together when
    /// their total would leave the wide columns less than their share of the
    /// table (GH #240).
    pub(crate) fn default_percent(self) -> Option<u8> {
        match self {
            Self::Narrow => Some(NARROW_DEFAULT_PERCENT),
            Self::Wide | Self::Rem(_) | Self::Percent(_) => None,
        }
    }

    /// The `style` attribute value an **explicit** declaration emits, or
    /// `None` for [`Wide`](Self::Wide), which declares nothing, and for
    /// [`Narrow`](Self::Narrow), whose share the renderer resolves against the
    /// rest of the table.
    pub(crate) fn explicit_css(self) -> Option<Cow<'static, str>> {
        match self {
            Self::Rem(rem) => Some(Cow::Owned(format!("width: {rem}rem"))),
            Self::Percent(percent) => Some(Cow::Owned(format!("width: {percent}%"))),
            Self::Wide | Self::Narrow => None,
        }
    }
}

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
///
/// A projection that reads a **relation** declares it with [`Self::needs`],
/// because the closure is opaque to the framework and the export builds its
/// query from those declarations (GH #177). The `(unloaded)` guard in the
/// closure is what catches a missed declaration, in test builds, at render.
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
    /// The width this column claims in the table's fixed layout (GH #240).
    width: ColumnWidth,
    /// Relations this column's projection reads, in the resource's vocabulary
    /// (GH #177).
    needs: Vec<&'static str>,
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
            width: ColumnWidth::Wide,
            needs: Vec::new(),
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
            width: ColumnWidth::Narrow,
            needs: Vec::new(),
        }
    }

    /// Declare the relations this column's projection reads (GH #177), under
    /// the names the resource's
    /// [`export_query`](super::Resource::export_query) matches on:
    /// `.needs(["author"])` for `|p| p.author.get().name.clone()`.
    ///
    /// **Declare every relation the closure reads.** A missing name is not a
    /// compile error: the export's query arrives without that relation, and
    /// the closure's `is_unloaded` guard — the unloaded-relation contract of
    /// ADR-0011, `"(unloaded)"` plus a `debug_assert!` — is what turns it
    /// into a loud failure instead of a silent `"-"`. Declaring a name nothing
    /// reads is harmless.
    ///
    /// Repeat calls accumulate: `.needs(["author"]).needs(["comments"])`.
    pub fn needs(mut self, names: impl IntoIterator<Item = &'static str>) -> Self {
        self.needs.extend(names);
        self
    }

    /// The relations this column declared, in declaration order. Internal: the
    /// public read is [`Table::include_needs`](super::Table::include_needs),
    /// the union the export hands its resource.
    pub(crate) fn include_names(&self) -> &[&'static str] {
        &self.needs
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

    /// Declare this column's width in the table's fixed layout (GH #240):
    /// `.width(ColumnWidth::Percent(20))` for a column that knows its own
    /// measure.
    ///
    /// The default follows the column's kind — see [`ColumnWidth`]. Override
    /// it when the content disagrees with the kind: a `computed` column that
    /// holds a name or a title is [`Wide`](ColumnWidth::Wide), a `String` field
    /// that holds a status is [`Narrow`](ColumnWidth::Narrow).
    pub fn width(mut self, width: ColumnWidth) -> Self {
        self.width = width;
        self
    }

    /// The width this column declares, which the renderer emits on its `th`
    /// and on every `td` of its column (GH #240).
    pub fn column_width(&self) -> ColumnWidth {
        self.width
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
    /// `escape_like_pattern` makes the term literal — a `%` or `_` the user
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
            .field("width", &self.width)
            .field("needs", &self.needs)
            .finish_non_exhaustive()
    }
}

/// Convert a single column or tuple of columns into `Vec<TextColumn<M>>`.
///
/// Tuple members are `TextColumn<M>` themselves: the `Into` bounds these impls
/// once carried existed for the removed `Column<M>` enum, and nothing else ever
/// implemented `From<_> for TextColumn<M>` but the reflexive impl (GH #228).
///
/// 5-tuple limit: without variadic generics this is idiomatic Rust — one arity
/// past `IntoSchema` in `schema/tree.rs`, which stops at four. Tables wider
/// than five columns are rare in admin UIs; extend (or macro-ify) when a real
/// Resource needs it.
pub trait IntoColumns<M> {
    fn into_columns(self) -> Vec<TextColumn<M>>;
}

impl<M> IntoColumns<M> for TextColumn<M> {
    fn into_columns(self) -> Vec<TextColumn<M>> {
        vec![self]
    }
}

impl<M> IntoColumns<M> for (TextColumn<M>, TextColumn<M>) {
    fn into_columns(self) -> Vec<TextColumn<M>> {
        vec![self.0, self.1]
    }
}

impl<M> IntoColumns<M> for (TextColumn<M>, TextColumn<M>, TextColumn<M>) {
    fn into_columns(self) -> Vec<TextColumn<M>> {
        vec![self.0, self.1, self.2]
    }
}

impl<M> IntoColumns<M> for (TextColumn<M>, TextColumn<M>, TextColumn<M>, TextColumn<M>) {
    fn into_columns(self) -> Vec<TextColumn<M>> {
        vec![self.0, self.1, self.2, self.3]
    }
}

impl<M> IntoColumns<M>
    for (
        TextColumn<M>,
        TextColumn<M>,
        TextColumn<M>,
        TextColumn<M>,
        TextColumn<M>,
    )
{
    fn into_columns(self) -> Vec<TextColumn<M>> {
        vec![self.0, self.1, self.2, self.3, self.4]
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

    /// GH #240: a column's kind picks its default width, `.width(..)`
    /// overrides it, and the declaration reaches the renderer as data — the
    /// CSS it writes on the `th`/`td`, never a Tailwind class.
    #[test]
    fn text_column_width_defaults_by_kind() {
        let field = TextColumn::r#for(User::fields().name(), |u| u.name.clone());
        assert_eq!(field.column_width(), ColumnWidth::Wide);

        let computed = TextColumn::computed("Status", |u: &User| u.name.clone());
        assert_eq!(computed.column_width(), ColumnWidth::Narrow);

        let declared = computed.width(ColumnWidth::Percent(30));
        assert_eq!(declared.column_width(), ColumnWidth::Percent(30));

        // A wide column declares nothing at all: it takes the share the
        // declared columns leave.
        assert!(ColumnWidth::Wide.explicit_css().is_none());
        assert!(ColumnWidth::Wide.default_percent().is_none());

        // A kind default is a nominal share of the table, resolved by the
        // renderer; an explicit width is emitted as written.
        assert_eq!(ColumnWidth::Narrow.default_percent(), Some(10));
        assert!(ColumnWidth::Narrow.explicit_css().is_none());
        assert_eq!(
            ColumnWidth::Rem(14).explicit_css().as_deref(),
            Some("width: 14rem")
        );
        assert_eq!(
            ColumnWidth::Percent(30).explicit_css().as_deref(),
            Some("width: 30%")
        );
    }

    /// GH #177: a column that reads no relation declares nothing, and repeat
    /// `.needs(..)` calls accumulate in declaration order.
    #[test]
    fn text_column_include_declarations_accumulate() {
        let plain = TextColumn::r#for(User::fields().name(), |u| u.name.clone());
        assert!(plain.include_names().is_empty());

        let declared = TextColumn::computed("Audit", |u: &User| u.name.clone())
            .needs(["author"])
            .needs(["comments", "post"]);
        assert_eq!(declared.include_names(), ["author", "comments", "post"]);
    }

    /// GH #177: the gathered set is what a resource's `export_query` asks, so
    /// membership and the empty case are the whole contract.
    #[test]
    fn include_needs_gathers_declarations() {
        let needs: IncludeNeeds = ["author", "comments"].into_iter().collect();
        assert!(needs.wants("author") && needs.wants("comments"));
        assert!(!needs.wants("post"));
        assert!(!needs.is_empty());

        // The whole set up front, as a resource's `query` declares it.
        let declared = IncludeNeeds::from(["author", "comments"]);
        assert!(declared.wants("author") && declared.wants("comments"));

        assert!(IncludeNeeds::default().is_empty());
        assert!(!IncludeNeeds::default().wants("author"));
    }
}
