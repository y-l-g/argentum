//! `Resource` — maps one Toasty [`Model`] to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, pages, and navigation. See
//! `CONTEXT.md` and ADR-0002.

use std::marker::PhantomData;

use toasty::stmt::List;
use topcoat::context::Cx;

use crate::schema::Schema;

/// Marker for the table description of a `Resource`'s list view.
///
/// Phase 1: stub. Later phases expand this into the full column/filter/search
/// DSL that drives the Toasty query in `docs/adr/0001-typed-field-lenses.md`.
#[derive(Debug, Default)]
pub struct Table<M> {
    _marker: PhantomData<M>,
}

impl<M> Table<M> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
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
    fn table() -> Table<Self::Model> {
        Table::new()
    }

    /// Description of the form/infolist. Phase 1: stub.
    fn form() -> Schema {
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
        let _table = UserResource::table();
        let _form = UserResource::form();
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
}
