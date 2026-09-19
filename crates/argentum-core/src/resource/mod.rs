//! `Resource` — maps one Toasty [`Model`](toasty::schema::Model) to its admin UI.
//!
//! One `Model` → one `Resource`. The trait is the single seam for query
//! scoping (`query`), form/table stubs, and navigation. See
//! `CONTEXT.md` and ADR-0002.
//!
//! Facade over the cohesive submodules split out in GH #133: `filter`,
//! `column`, `state`, `table` (+ `table::render`/`table::export`),
//! `navigation`, and `naming`. The re-export surface is unchanged.

use std::collections::HashMap;

use toasty::stmt::List;
use topcoat::Result;
use topcoat::context::Cx;

use crate::schema::Schema;

mod column;
mod filter;
mod naming;
mod navigation;
mod state;
mod table;

pub use column::{Column, IntoColumns, TextColumn};
pub use filter::{DateFilter, Filter, IntoFilters, SelectFilter, TernaryFilter, VariantFilter};
pub use navigation::{HrefCheck, NavTarget, NavigationItem};
#[cfg(test)]
pub(crate) use state::MAX_QUERY_TERM;
pub(crate) use state::clamp_query_term;
pub use state::{Sort, TablePage, TableSignals, TableState};
pub use table::{GroupDef, GroupKey, RowKey, Table};

use naming::{kebab_case, pluralize, type_short_name};

/// Maps one Toasty `Model` to its admin UI.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    ///
    /// `Send + Sync` holds for every data-only model struct and is required
    /// for concurrent rendering of the resource's pages.
    type Model: toasty::schema::Model + Send + Sync + 'static;

    /// Whether the current user may view the list page.
    ///
    /// Also gates relationship option loads (GH #108): a related resource
    /// that denies this cannot offer its records as options at all.
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may view the given record.
    ///
    /// Checked on the edit page (GET), the edit POST (which requires both
    /// `can_view` and `can_update`, GH #86), per row in CSV export, and on
    /// each record behind a relationship `Select`'s options (GH #108). Note
    /// both hooks default-deny: a resource used as a relationship target
    /// must allow `can_view_any` **and** `can_view` (overriding one does not
    /// imply the other). The list page deliberately checks only
    /// `can_view_any` (GH #86): `can_view` is an in-memory Rust predicate
    /// that cannot run in SQL, and filtering rows after cursor pagination
    /// would mislabel pages (holes, wrong Next/Prev). Row-level visibility
    /// that must hold on the list belongs in [`Self::query`] (the tenancy
    /// seam, ADR-0002), which every loader — list, edit, delete, bulk,
    /// export — already funnels through.
    fn can_view(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether the current user may create a new record.
    fn can_create(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may update the given record.
    fn can_update(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether the current user may delete the given record.
    ///
    /// Checked on the single-delete and bulk-delete POSTs together with
    /// `can_view` (GH #168) — the edit contract: a record that cannot be
    /// viewed cannot be deleted by UUID-guessing the route.
    fn can_delete(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether this resource exposes row and bulk delete chrome (GH #96).
    ///
    /// The default renders Delete buttons and the bulk bar; server policy
    /// (`can_delete`) still denies regardless. Read-only resources should
    /// override to `false` so users never reach a 403 after a confirmation
    /// round-trip.
    fn deletable() -> bool {
        true
    }

    /// Whether this resource exposes row edit chrome (GH #162).
    ///
    /// The default renders an `Edit` link per row (Filament's `recordActions`
    /// `EditAction`); server policy (`can_view` + `can_update`) still denies
    /// in the edit GET/POST regardless. Read-only resources should override
    /// to `false` alongside [`Self::deletable`].
    fn editable() -> bool {
        true
    }

    /// The URL slug for this resource's pages, e.g. `"users"` mounts the list
    /// at `{panel prefix}/users`.
    ///
    /// Defaults to the Filament convention (`HasRoutes::resolveDefaultSlug`):
    /// take the resource type's name, strip a trailing `Resource`, pluralize
    /// (`UserResource` → `Users`, `CategoryResource` → `Categories`), then
    /// kebab-case (`BlogPostResource` → `blog-posts`). Override for irregular
    /// naming the rules cannot guess (`UsersResource` pluralizes to
    /// `userses` — name resources singular, or override).
    fn slug() -> String {
        let name = type_short_name::<Self>();
        let singular = name.strip_suffix("Resource").unwrap_or(name);
        kebab_case(&pluralize(singular))
    }

    /// The sidebar label, e.g. `"Users"`.
    ///
    /// Defaults to the pluralized `Model` type name (Filament's plural model
    /// label): `User` → `Users`, `Category` → `Categories`, `Person` →
    /// `People`. Override for custom wording.
    fn navigation_label() -> String {
        pluralize(type_short_name::<Self::Model>())
    }

    /// Base query — the **single seam** for tenancy/soft-delete scoping
    /// (ADR-0002). Every loader starts from this query.
    ///
    /// Returns the raw typed statement query (the spec's original signature):
    /// raw queries compose generically — `filter`, `order_by`, and
    /// `Paginate::new` are available on the raw form for any `M: Model` —
    /// which is what lets [`crate::panel::Panel`] drive every resource's list
    /// page through one handler. Scoping it via `Model::filter(..)` in an
    /// override stays as ergonomic as before; the wrapper's extra methods are
    /// only needed by hand-written loaders.
    fn query(_cx: &Cx) -> toasty::stmt::Query<List<Self::Model>> {
        toasty::stmt::Query::<List<Self::Model>>::all()
    }

    /// Whether this resource requires a tenant in every handler (GH #87).
    ///
    /// Opt-in and default-open today: `false` preserves the current behavior
    /// (unscoped `Resource::query` default). Resources with a `tenant_id`
    /// column should override to `true` so a missing tenant fails closed
    /// (403) instead of leaking unscoped rows or minting nil-tenant orphans.
    fn requires_tenant() -> bool {
        false
    }

    /// Description of the list view.
    ///
    /// The default is empty, and an empty table **cannot render**: the
    /// default `Resource` is not listable until it declares columns via
    /// `Table::columns(..)` and a row key via `Table::id(..)` (see
    /// [`Table::render`]).
    fn table(_cx: &Cx) -> Table<Self::Model> {
        Table::new()
    }

    /// Description of the form/infolist. Phase 1: stub.
    fn form(_cx: &Cx) -> Schema {
        Schema::empty()
    }

    /// Sidebar entry for the resource.
    ///
    /// The default declares a label ([`Self::navigation_label`]) and no URL:
    /// the Panel that owns the resource resolves where it is mounted, so this
    /// entry never links at a mount the resource guessed (GH #165).
    ///
    /// Override to curate this resource's sidebar entry: `Panel::resource`
    /// consumes the result through the panel-aware navigation seam, so a
    /// `.sorted(..)` order, a custom label or a typed
    /// [`NavigationItem::from_href`] item all take effect. Decorate the default
    /// with [`NavigationItem::for_resource`]
    /// (`NavigationItem::for_resource::<Self>().sorted(-1)`) to keep the
    /// panel-owned URL; spell a URL out yourself ([`NavigationItem::at`]) only
    /// to link somewhere other than this resource's list page — the Panel keeps
    /// such a URL verbatim.
    fn navigation() -> NavigationItem {
        NavigationItem::for_resource::<Self>()
    }

    /// Create a new record from form values.
    ///
    /// The `Panel` create handler validates `required`/`email` inline and checks
    /// `Resource::can_create` before calling this, inside a framework-owned
    /// transaction (GH #84): `ex` is the open tx — run every statement
    /// through it (`exec(&mut *ex)`) and never open a second handle, so the
    /// write commits atomically with the handler's checks. The default
    /// implementation returns an error; resources should override to perform
    /// the actual `toasty::create!` (or `Insert`).
    fn create_record(
        _cx: &Cx,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "create not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Update the already-authorized `record` from form values (GH #86).
    ///
    /// The handler loads `record` through the tenancy-scoped query **inside
    /// the framework transaction** and checks `can_view` + `can_update` on
    /// that snapshot before calling this — use the passed record directly,
    /// never re-query by id (re-loading outside the checked snapshot was the
    /// TOCTOU hole). Run writes through `ex`; commit/rollback is the
    /// handler's job. Residual (documented, not fixed): a concurrent
    /// cross-transaction policy flip landing between this tx's snapshot and
    /// its commit is backend-isolation territory, out of scope here.
    fn update_record(
        _cx: &Cx,
        _record: Self::Model,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "update not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Delete the already-authorized `record` (GH #84, #86): same checked-
    /// snapshot contract as [`Self::update_record`] — no re-query, write
    /// through `ex`.
    fn delete_record(
        _cx: &Cx,
        _record: Self::Model,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "delete not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Bulk-delete the already-authorized `records` (GH #84): the handler
    /// fetches through the tenancy-scoped `IN` query inside the framework
    /// transaction and checks `can_delete` on every row before calling this.
    /// Delete them through `ex` — any error rolls the whole batch back, so
    /// mid-loop failures delete zero rows.
    fn bulk_delete_records(
        _cx: &Cx,
        _records: Vec<Self::Model>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            Err(std::io::Error::other(format!(
                "bulk delete not implemented for {}",
                std::any::type_name::<Self>()
            ))
            .into())
        }
    }

    /// Hydrate form values from a record for the Edit page.
    /// Default returns empty; resources should override to return field->value
    /// mappings (e.g. `name -> user.name`).
    fn hydrate_form_values(_record: &Self::Model) -> HashMap<String, String> {
        HashMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::Db;
    use topcoat::context::CxTestBuilder;

    #[derive(Debug, Clone, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct UserResource;

    impl Resource for UserResource {
        type Model = User;

        fn query(_cx: &Cx) -> toasty::stmt::Query<List<User>> {
            // Custom scoping example: only users named Ada
            toasty::stmt::Query::<List<User>>::all().filter(User::fields().name().eq("Ada"))
        }
    }

    struct BareResource;

    impl Resource for BareResource {
        type Model = User;
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
