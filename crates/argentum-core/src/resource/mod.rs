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
mod commit;
mod filter;
mod naming;
mod navigation;
mod relation;
mod state;
mod table;

pub use column::{IncludeNeeds, IntoColumns, TextColumn};
pub(crate) use commit::run_after_commit;
pub use commit::{Committed, Mutation};
pub use filter::{DateFilter, Filter, IntoFilters, SelectFilter, TernaryFilter, VariantFilter};
pub use navigation::{NavTarget, NavigationItem};
pub use relation::{IntoRelationColumns, RelationColumn, RelationColumns, render_relation};
#[cfg(test)]
pub(crate) use state::MAX_QUERY_TERM;
pub(crate) use state::clamp_query_term;
pub use state::{Sort, TablePage, TableSignals, TableState};
pub(crate) use state::{cursor_after, cursor_before, cursor_none, split_cursor};
pub use table::{GroupDef, GroupKey, RowKey, Table};

use naming::{kebab_case, pluralize, type_short_name};

/// Maps one Toasty `Model` to its admin UI.
///
/// # Contract (GH #138)
///
/// **Every method has a default**, so a resource compiles the moment it
/// declares a [`Model`](Self::Model) — and an omission must therefore fail
/// loudly rather than silently:
///
/// - **Checked at [`Panel::build`](crate::panel::Panel::build)**, which returns
///   `Err` naming the type: the grid must be renderable
///   ([`table`](Self::table) declares columns and a row key) and, where
///   [`can_create`](Self::can_create) allows it, the
///   [`form`](Self::form) must declare fields. `table`, `form` and `can_create`
///   are declarations: they must not need request-scoped context, because the
///   panel calls them once at boot with the app's values and no request.
/// - **Loud at request time**: the record fns
///   ([`create_record`](Self::create_record),
///   [`update_record`](Self::update_record),
///   [`delete_record`](Self::delete_record),
///   [`bulk_delete_records`](Self::bulk_delete_records)) default to an error
///   naming the type, so a resource that never implemented delete answers
///   "delete not implemented for …" instead of writing nothing quietly.
/// - **Claimed by the flags**: [`deletable`](Self::deletable) and
///   [`editable`](Self::editable) default to `true`, which both registers the
///   routes and renders the chrome. A read-only resource overrides them to
///   `false`; that is the declaration that it has nothing to implement.
/// - **Default-deny is untouched**: every `can_*` still defaults to `false`, so
///   an unconfigured resource exposes no data and no mutation.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    ///
    /// `Send + Sync` holds for every data-only model struct and is required
    /// for concurrent rendering of the resource's pages.
    ///
    /// `Clone` is part of the contract because a committed mutation names its
    /// rows (GH #112): a handler keeps a copy of the rows it loaded while the
    /// record fn consumes them, so the hook can be handed what was written.
    type Model: toasty::schema::Model + Send + Sync + Clone + 'static;

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

    /// How one record is displayed on the detail page (GH #187), read-only.
    ///
    /// The same [`Schema`](crate::schema::Schema) a form uses, rendered for
    /// reading: a `TextInput` shows its stored value instead of an `<input>`,
    /// a `Select` shows the option label the form offered, and a layout block
    /// keeps the structure it declares (`Grid` stays a grid). Declaring a view
    /// is what turns the detail page on — the default declares nothing, so a
    /// resource that does not override this renders no page content, links no
    /// `View` row action, and 404s the route.
    ///
    /// Values come from [`hydrate_form_values`](Self::hydrate_form_values), so
    /// a field bound here is one the resource already knows how to read. A
    /// relation is not one of these fields — it is a list of records, not a
    /// string — and renders through
    /// [`view_relations`](Self::view_relations), which the detail page draws
    /// under this Schema.
    ///
    /// Read-only is a promise, not a disabled form: nothing here validates or
    /// submits, and no field renders a required marker or an error slot —
    /// including a `Repeater`, which renders its label over its children's
    /// values.
    fn view(_cx: &Cx) -> crate::schema::Schema {
        crate::schema::Schema::empty()
    }

    /// The related records on this resource's detail page (GH #187).
    ///
    /// [`view`](Self::view) is a `Schema`, and a `Schema` is rendered from the
    /// record's *string projection* — one `HashMap<String, String>` — because
    /// that is what every field binds. A relation is not a string and may be a
    /// list of records, so it cannot ride that map; this hook is where it
    /// renders instead, with the loaded record in hand.
    ///
    /// The reason is structural, not stylistic: the render tree is monomorphic
    /// (one walk renders every resource) while a record's type is the
    /// resource's, so a `Schema` node cannot be handed `&Self::Model` without
    /// type erasure across a borrow — the workspace denies `unsafe`, and the
    /// safe erasures either require a `'static` record or capture the caller's
    /// locals in the render's lifetime. A generic method has none of those
    /// problems and keeps the typing.
    ///
    /// This is the half that makes `Resource::query`'s `include` pay: the
    /// related rows are already loaded on the record, so a hook that reads
    /// `record.comments.get()` issues no query at all. Touching an
    /// un-included relation panics (`Deferred::get`), and the way to notice is
    /// [`Deferred::is_unloaded`](toasty::Deferred::is_unloaded) — the same
    /// marker the list columns check.
    ///
    /// Returns `None` (the default) for a resource with no related records to
    /// show, which renders nothing.
    /// The two lifetimes are deliberately separate: the returned view may
    /// borrow the request context, never the record. A view that held the
    /// record would pin the handler's local binding for as long as the page,
    /// which does not compile — the renderer's projections return owned
    /// strings ([`render_relation`]), so nothing needs to.
    fn view_relations<'a>(
        _cx: &'a Cx,
        _record: &Self::Model,
    ) -> Option<topcoat::view::BoxView<'a>> {
        None
    }

    /// Whether this resource declares a detail page (GH #187).
    ///
    /// Derived from [`view`](Self::view) rather than declared twice, so the
    /// route and the row link cannot disagree with the schema that renders
    /// them. The detail handler uses it to 404 a resource that declares
    /// nothing, and the row chrome uses it to leave the link off.
    fn viewed(cx: &Cx) -> bool {
        !Self::view(cx).is_empty()
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
    ///
    /// # Keep unique constraints in step with this scope (GH #88)
    ///
    /// The app-side unique pre-check probes submitted values **through this
    /// query**, so it only sees the rows this query returns. A `#[unique]` index
    /// *broader* than the scope is therefore invisible to it: the probe misses
    /// the colliding row, the database refuses the write, and the user gets a
    /// 500 instead of the inline "has already been taken".
    ///
    /// The tenant case is the one that bites — scoping to `tenant_id` here while
    /// the column carries a plain `#[unique]` (global) makes two tenants sharing
    /// a value a legitimate pair to the probe and a constraint violation to the
    /// database. Scope the constraint to match: `#[unique(tenant_id, email)]`,
    /// which also makes it say what it means. `Author.email` in the showcase is
    /// the worked example.
    ///
    /// The invariant cannot be *checked* at declaration time — a query's filters
    /// are not introspectable, so nothing can compare the two automatically.
    /// Upstream #117 (a driver-level unique-violation predicate) is what would
    /// make a mismatch safe rather than merely documented.
    fn query(_cx: &Cx) -> toasty::stmt::Query<List<Self::Model>> {
        toasty::stmt::Query::<List<Self::Model>>::all()
    }

    /// The CSV export's base query: [`Self::query`], narrowed to the relations
    /// the rendered columns declared (GH #177).
    ///
    /// The export writes a cell per column, so it asks the table which
    /// relations those columns' projections read ([`Table::include_needs`] —
    /// each column declares them with [`TextColumn::needs`]) and hands the
    /// answer here. Overriding this is how a resource stops paying for
    /// includes it carries for other pages: a relation `query` loads for the
    /// detail page or the live list rides along on an export only when a
    /// rendered column declared it.
    ///
    /// **The default ignores `needs` and returns [`Self::query`] unchanged**,
    /// so a resource that overrides nothing behaves exactly as before.
    /// Over-fetching a relation nothing reads costs a join; dropping one a
    /// column does read breaks the render — the default takes the safe side.
    ///
    /// A resource splits its base query in two, one branch per declared name:
    ///
    /// ```ignore
    /// impl PostResource {
    ///     fn base(cx: &Cx, needs: &IncludeNeeds) -> Query<List<Post>> {
    ///         let mut q = /* tenancy filter, as in query() */;
    ///         if needs.wants("author") { q = q.include(inc_author); }
    ///         if needs.wants("comments") { q = q.include(inc_comments); }
    ///         q
    ///     }
    /// }
    ///
    /// fn query(cx: &Cx) -> Query<List<Post>> {
    ///     Self::base(cx, &IncludeNeeds::from(["author", "comments"]))
    /// }
    ///
    /// fn export_query(cx: &Cx, needs: &IncludeNeeds) -> Query<List<Post>> {
    ///     Self::base(cx, needs)
    /// }
    /// ```
    ///
    /// # What an override must keep
    ///
    /// - **The scope of [`Self::query`].** This is the same tenancy/soft-delete
    ///   seam (ADR-0002), and the export is a reader like any other: an
    ///   override that drops the filter exports other tenants' rows.
    /// - **Whatever the policy path reads.** The export's visibility scan calls
    ///   [`Self::can_view`] on every row of both passes, before any cell is
    ///   written, so a `can_view` that reads a relation needs that relation
    ///   included even though no column declared it — include it
    ///   unconditionally in the narrowed branch. Reading an un-included
    ///   relation panics in `Deferred::get`.
    /// - **Every name a column declared.** A declared name with no matching
    ///   include renders an unloaded relation, which the column's `is_unloaded`
    ///   guard (ADR-0011) reports in test builds instead of a silent `"-"`.
    ///
    /// The **list page does not use this**: it keeps inheriting [`Self::query`]
    /// (the #172 grill's decision 10), so only the export's constant-factor
    /// over-fetch is addressed here.
    fn export_query(cx: &Cx, _needs: &IncludeNeeds) -> toasty::stmt::Query<List<Self::Model>> {
        Self::query(cx)
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
    /// consumes the result through the panel-aware navigation seam, so a custom
    /// `order` or a custom label takes effect. Decorate the default with
    /// [`NavigationItem::for_resource`]
    /// (`NavigationItem { order: -1, ..NavigationItem::for_resource::<Self>() }`)
    /// to keep the panel-owned URL; spell a URL out yourself
    /// ([`NavigationItem::at`]) only to link somewhere other than this
    /// resource's list page — the Panel keeps such a URL verbatim.
    fn navigation() -> NavigationItem {
        NavigationItem::for_resource::<Self>()
    }

    /// Create a new record from form values, returning the row it wrote.
    ///
    /// The `Panel` create handler validates `required`/`email` inline and checks
    /// `Resource::can_create` before calling this, inside a framework-owned
    /// transaction (GH #84): `ex` is the open tx — run every statement
    /// through it (`exec(&mut *ex)`) and never open a second handle, so the
    /// write commits atomically with the handler's checks. The default
    /// implementation returns an error; resources should override to perform
    /// the actual `toasty::create!` (or `Insert`).
    ///
    /// Return the created row — `toasty::create!` already hands it back, and
    /// the framework cannot otherwise name what a create wrote: the primary key
    /// is the database's (or the app's) to generate, so it is only knowable
    /// from the row. That is what [`Self::after_commit`] receives for a create
    /// (GH #112).
    fn create_record(
        _cx: &Cx,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<Self::Model>> + Send
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

    /// Update the already-authorized `record` from form values (GH #86),
    /// returning the row as it now stands.
    ///
    /// The handler loads `record` through the tenancy-scoped query **inside
    /// the framework transaction** and checks `can_view` + `can_update` on
    /// that snapshot before calling this — use the passed record directly,
    /// never re-query by id (re-loading outside the checked snapshot was the
    /// TOCTOU hole). Run writes through `ex`; commit/rollback is the
    /// handler's job. Residual (documented, not fixed): a concurrent
    /// cross-transaction policy flip landing between this tx's snapshot and
    /// its commit is backend-isolation territory, out of scope here.
    ///
    /// Return the updated row, the way [`Self::create_record`] returns the
    /// created one: `toasty::update!` already resolves to it, and
    /// [`Self::after_commit`] needs what was written rather than what was
    /// loaded — the pre-write snapshot would have a watcher notify its
    /// subscribers with stale values (GH #112).
    fn update_record(
        _cx: &Cx,
        _record: Self::Model,
        _values: HashMap<String, String>,
        _ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<Self::Model>> + Send
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

    /// Post-commit work for a mutation this resource committed (GH #112).
    ///
    /// The one place a side effect that must not survive a rollback belongs —
    /// an email, a webhook, an audit row, cache invalidation. Called by the
    /// framework **once per successful write**, after `tx.commit()` and before
    /// the response: running it inside a record fn would leak the effect when
    /// the transaction rolls back, and the write handlers' pool discipline
    /// (GH #84) forbids a second handle while the transaction is open. By the
    /// time this runs the transaction is gone, so it may open its own `Db`
    /// handle — `db(cx)` — or none at all.
    ///
    /// [`Committed`] names the mutation and the rows it wrote: the row a create
    /// returned, the row an update returned (the committed state, not the
    /// snapshot the handler loaded), the rows a delete or bulk delete removed —
    /// deletes are the one case where the row is gone by the time you see it.
    /// A bulk delete is **one call** with every row, not one call per row.
    ///
    /// It is never called when nothing committed: a validation error, a policy
    /// denial, a failed record fn, or a failed commit all leave the hook
    /// untouched, so a rollback can never produce the effect.
    ///
    /// A hook that returns `Err` is logged and ignored — the write is
    /// committed, and an error page would misreport it. Retries and delivery
    /// guarantees are the app's to build (an outbox written here is the usual
    /// shape); the framework promises neither. A hook that panics is a bug in
    /// the hook and surfaces as Topcoat's panic-isolated 500.
    ///
    /// Defaults to a no-op, so a resource that declares nothing is unaffected.
    ///
    /// ```ignore
    /// impl Resource for PostResource {
    ///     type Model = Post;
    ///
    ///     async fn after_commit(cx: &Cx, committed: Committed<Post>) -> Result<()> {
    ///         // The transaction is committed and gone, so this opens its own handle.
    ///         let mut db = db(cx);
    ///         for post in committed.records() {
    ///             notify_watchers(post, committed.mutation()).await?;
    ///         }
    ///         Ok(())
    ///     }
    /// }
    /// ```
    fn after_commit(
        _cx: &Cx,
        _committed: Committed<Self::Model>,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move { Ok(()) }
    }

    /// Hydrate form values from a record for the Edit and View pages.
    ///
    /// The record's **string projection**: the flat map every field binds, keyed
    /// by the name the control posts. Default returns empty; a resource
    /// overrides it to map its record to those keys (`name -> record.name`).
    ///
    /// `cx` carries the app schema (GH #191). A scalar projection needs no
    /// request context, but an embedded **value** does: its keys are the
    /// columns the compiled mapping resolves
    /// ([`write_embedded`](crate::schema::write_embedded)), and re-deriving
    /// those names here is exactly what GH #185 removed. The context is the
    /// request's, the same one `form(cx)` and the record fns receive.
    fn hydrate_form_values(_cx: &Cx, _record: &Self::Model) -> HashMap<String, String> {
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
