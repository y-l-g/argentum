//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, pages, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;

use toasty::stmt::{Expr, List, OrderByExpr};
use topcoat::context::Cx;
use topcoat::{Result, view::*};

use crate::schema::{FieldLens, Schema};

/// Text column bound to a typed lens. `TextColumn::for(User::fields().name())`
/// fails if the column does not exist (ADR-0001).
#[derive(Debug, Clone)]
pub struct TextColumn<M> {
    path: FieldLens<M, String>,
    name: String,
    label: String,
    searchable: bool,
    sortable: bool,
}

impl<M> TextColumn<M>
where
    M: toasty::schema::Model,
{
    pub fn for_lens(path: FieldLens<M, String>) -> Self {
        let stmt_path: toasty_core::stmt::Path = path.clone().into();
        debug_assert!(
            !stmt_path.projection.as_slice().is_empty(),
            "TextColumn::for expects a field lens, got root path"
        );
        let idx = stmt_path
            .projection
            .as_slice()
            .first()
            .copied()
            .expect("field lens must have a projection");
        let model = M::schema();
        let field_name = model
            .fields()
            .get(idx)
            .unwrap_or_else(|| {
                panic!(
                    "field index {idx} out of bounds for {}",
                    std::any::type_name::<M>()
                )
            })
            .name
            .app_unwrap()
            .to_string();
        let label = {
            let mut c = field_name.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        };
        Self {
            path,
            name: field_name,
            label,
            searchable: false,
            sortable: false,
        }
    }

    pub fn r#for(path: FieldLens<M, String>) -> Self {
        Self::for_lens(path)
    }

    pub fn searchable(mut self) -> Self {
        self.searchable = true;
        self
    }

    pub fn sortable(mut self) -> Self {
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

    pub fn header_class(&self) -> String {
        let mut cls = "ac-column".to_string();
        if self.searchable {
            cls.push_str(" ac-column--searchable");
        }
        if self.sortable {
            cls.push_str(" ac-column--sortable");
        }
        cls
    }

    pub fn to_search_expr(&self, term: &str) -> Option<Expr<bool>> {
        let t = term.trim();
        if !self.searchable || t.is_empty() {
            return None;
        }
        Some(self.path.clone().starts_with(t.to_string()))
    }

    pub fn to_order_by(&self) -> Option<OrderByExpr> {
        if self.sortable {
            // Slice 2: single asc; PK tie-breaker will be added in slice 3 for stable pagination
            Some(self.path.clone().asc())
        } else {
            None
        }
    }
}

/// Column enum — Slice 2: only `Text`. Will generalize to `Number`, `Badge`, etc. in later slices.
#[derive(Debug, Clone)]
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

    pub fn header_class(&self) -> String {
        match self {
            Column::Text(c) => c.header_class(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Column::Text(c) => &c.name,
        }
    }
}

/// Convert a single column or tuple of columns into `Vec<Column<M>>`.
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

/// Table description of a `Resource`'s list view. Declares columns and how they map to queries.
#[derive(Debug)]
pub struct Table<M> {
    columns: Vec<Column<M>>,
    _marker: PhantomData<M>,
}

impl<M> Default for Table<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Table<M> {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Create a table for the given model. `cx` is reserved for future tenancy/policy scoping.
    pub fn r#for(_cx: &Cx) -> Self {
        Self::new()
    }

    /// Declare columns. Accepts a single column or tuple of columns.
    pub fn columns(mut self, cols: impl IntoColumns<M>) -> Self {
        self.columns = cols.into_columns();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Global search predicate — OR across searchable columns (portable `starts_with`).
    pub fn search_expr(&self, term: &str) -> Option<Expr<bool>>
    where
        M: toasty::schema::Model,
    {
        let t = term.trim();
        if t.is_empty() {
            return None;
        }
        let mut exprs = self.columns.iter().filter_map(|c| match c {
            Column::Text(col) => col.to_search_expr(t),
        });
        let first = exprs.next()?;
        Some(exprs.fold(first, |acc, e| acc.or(e)))
    }

    /// First sortable column's order_by (asc). Slice 3 will add PK tie-breaker.
    pub fn order_by(&self) -> Option<OrderByExpr>
    where
        M: toasty::schema::Model,
    {
        self.columns.iter().find_map(|c| match c {
            Column::Text(col) => col.to_order_by(),
        })
    }

    /// Render the table for the given rows. Header shows searchable/sortable indicators;
    /// rows are keyed by index (Slice 2: will use `row.id` when `M: HasId` in slice 3).
    pub async fn render(&self, cx: &Cx, rows: &[M]) -> Result<View>
    where
        M: toasty::schema::Model + std::fmt::Debug + Send + Sync + 'static,
    {
        let rows_dbg: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        view! {
            cx =>
            <table class="ac-table">
                <thead>
                    <tr>
                        for col in &self.columns {
                            <th class=(col.header_class())>(col.label())</th>
                        }
                    </tr>
                </thead>
                <tbody>
                    for (idx, row_dbg) in rows_dbg.iter().enumerate() {
                        <tr key=(idx.to_string())>
                            for col in &self.columns {
                                <td>(format!("{}: {}", col.name(), row_dbg))</td>
                            }
                        </tr>
                    }
                </tbody>
            </table>
        }
    }
}

/// Which pages a `Resource` exposes.
#[derive(Debug, Default)]
pub struct Pages<R> {
    _marker: PhantomData<R>,
}

impl<R> Pages<R> {
    /// The conventional CRUD set (list / create / edit / view). Phase 1: stub.
    pub fn crud() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Sidebar entry derived from a `Resource` (see `CONTEXT.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationItem {
    pub label: String,
    pub url: String,
}

impl NavigationItem {
    /// Derive a sidebar entry from a `Resource` type.
    ///
    /// Label is the `Model`'s type name without module path and with a
    /// trailing `s` for pluralisation (matching Filament's `User` → `Users`).
    /// URL is `/admin` for the single-resource Phase 1 shell; multi-resource
    /// routing will become `/admin/<kebab-plural>` (ADR-0002 query seam
    /// handles scoping, Panel prefix owns the mount point).
    pub fn from_resource<R: Resource>() -> Self {
        let model_name = std::any::type_name::<R::Model>();
        let short = model_name.rsplit("::").next().unwrap_or(model_name);
        let label = format!("{short}s");
        Self {
            label,
            url: "/admin".to_string(),
        }
    }
}

/// Maps one Toasty `Model` to its admin UI.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    type Model: toasty::schema::Model;

    /// Base query — the **single seam** for tenancy/soft-delete scoping
    /// (ADR-0002). Every loader starts from this query.
    fn query(_cx: &Cx) -> <Self::Model as toasty::schema::Model>::Query<List<Self::Model>>
    where
        Self::Model: toasty::schema::Model,
    {
        <Self::Model as toasty::schema::Model>::wrap_query(
            toasty::stmt::Query::<List<Self::Model>>::all(),
        )
    }

    /// Description of the list view. Phase 1: stub.
    fn table(_cx: &Cx) -> Table<Self::Model> {
        Table::new()
    }

    /// Description of the form/infolist. Phase 1: stub.
    fn form(_cx: &Cx) -> Schema {
        Schema::empty()
    }

    /// Which pages the resource exposes. Phase 1: the CRUD stub.
    fn pages() -> Pages<Self> {
        Pages::crud()
    }

    /// Sidebar entry for the resource.
    fn navigation() -> NavigationItem {
        NavigationItem::from_resource::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::Db;
    use topcoat::context::CxTestBuilder;

    #[derive(Debug, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct UserResource;

    impl Resource for UserResource {
        type Model = User;

        fn query(_cx: &Cx) -> <User as toasty::schema::Model>::Query<List<User>> {
            // Custom scoping example: only users named Ada
            User::filter(User::fields().name().eq("Ada"))
        }
    }

    struct BareResource;

    impl Resource for BareResource {
        type Model = User;
    }

    #[test]
    fn resource_associated_model_is_accessible() {
        fn assert_resource<R: Resource>() {}
        assert_resource::<UserResource>();
        assert_resource::<BareResource>();
    }

    #[test]
    fn navigation_derives_label_and_url_from_model() {
        let item = NavigationItem::from_resource::<UserResource>();
        assert_eq!(item.label, "Users");
        assert_eq!(item.url, "/admin");
    }

    #[test]
    fn default_query_returns_all() {
        let cx = CxTestBuilder::new().build();
        let _q = BareResource::query(&cx);
        // No panic — the default impl returns Model::all()
        let _q2 = UserResource::query(&cx);
    }

    #[test]
    fn table_form_pages_have_defaults() {
        let cx = CxTestBuilder::new().build();
        let _table = UserResource::table(&cx);
        let _form = UserResource::form(&cx);
        let _pages = UserResource::pages();
        let _nav = UserResource::navigation();
    }

    #[tokio::test]
    async fn query_seam_is_cloneable_via_db_helper() {
        // Proves the seam can be combined with the `db(cx)` helper from T2
        // without taking ownership of the query.
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(User { name: "Ada" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(User { name: "Bob" })
            .exec(&mut db)
            .await
            .unwrap();

        let cx = CxTestBuilder::new().app_context(db).build();
        let mut db = crate::db::db(&cx);
        let rows = UserResource::query(&cx).exec(&mut db).await.unwrap();
        // Custom query filters to Ada only
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");

        let rows_all = BareResource::query(&cx).exec(&mut db).await.unwrap();
        assert_eq!(rows_all.len(), 2);
    }

    // ---- Slice 2: TextColumn with typed lens (red) ----

    #[test]
    fn text_column_searchable_produces_starts_with() {
        let col = TextColumn::r#for(User::fields().name()).searchable();
        assert!(
            col.to_search_expr("Ada").is_some(),
            "searchable should produce expr"
        );
        assert!(
            TextColumn::r#for(User::fields().name())
                .to_search_expr("Ada")
                .is_none(),
            "non-searchable should be None"
        );
    }

    #[test]
    fn text_column_sortable_produces_order_by() {
        let col = TextColumn::r#for(User::fields().name()).sortable();
        assert!(
            col.to_order_by().is_some(),
            "sortable should produce order_by"
        );
        assert!(
            TextColumn::r#for(User::fields().name())
                .to_order_by()
                .is_none(),
            "non-sortable should be None"
        );
    }

    #[tokio::test]
    async fn table_for_columns_renders_with_keyed_rows() {
        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).searchable(),
            TextColumn::r#for(User::fields().name()).sortable(),
        ));
        // Use dummy rows for render check (no DB) — Slice 2: key is index, Slice 3 will use row.id
        let rows = vec![
            User {
                id: uuid::Uuid::new_v4(),
                name: "Ada".to_string(),
            },
            User {
                id: uuid::Uuid::new_v4(),
                name: "Bob".to_string(),
            },
        ];
        let html = table.render(&cx, &rows).await.unwrap().render(&cx);
        assert!(html.contains("ac-table"), "missing ac-table in {html}");
        assert!(
            html.contains("ac-column--searchable"),
            "missing ac-column--searchable in {html}"
        );
        assert!(
            html.contains("ac-column--sortable"),
            "missing ac-column--sortable in {html}"
        );
        assert!(html.contains("Name"), "missing Name header in {html}");
        for row in &rows {
            assert!(
                html.contains(&row.name),
                "missing row name {} in {html}",
                row.name
            );
        }
    }

    #[tokio::test]
    async fn table_search_filters_via_column() {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(User { name: "Ada" })
            .exec(&mut db)
            .await
            .unwrap();
        toasty::create!(User { name: "Bob" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let col = TextColumn::r#for(User::fields().name()).searchable();
        let expr = col.to_search_expr("Ada").unwrap();
        let mut db = crate::db::db(&cx);
        let rows = User::filter(expr).exec(&mut db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");
        // Empty term → None
        assert!(col.to_search_expr("").is_none());
        assert!(col.to_search_expr("   ").is_none());
    }

    #[test]
    fn table_search_expr_ors_across_searchable_columns() {
        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).searchable(),
            TextColumn::r#for(User::fields().name()).searchable(),
        ));
        assert!(table.search_expr("Ada").is_some());
        assert!(table.search_expr("").is_none());
        assert!(table.search_expr("   ").is_none());
        let table_none =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()));
        assert!(table_none.search_expr("Ada").is_none());
    }

    #[test]
    fn table_order_by_returns_first_sortable() {
        let cx = CxTestBuilder::new().build();
        let table = Table::<User>::r#for(&cx).columns((
            TextColumn::r#for(User::fields().name()).sortable(),
            TextColumn::r#for(User::fields().name()),
        ));
        assert!(table.order_by().is_some());
        let table_none =
            Table::<User>::r#for(&cx).columns(TextColumn::r#for(User::fields().name()));
        assert!(table_none.order_by().is_none());
    }
}
