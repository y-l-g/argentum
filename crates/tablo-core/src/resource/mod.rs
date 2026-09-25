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

use toasty::stmt::{List, Query};
use topcoat::{Result, context::Cx};

use crate::schema::Schema;

mod column;
mod commit;
mod filter;
mod naming;
mod navigation;
mod relation;
mod state;
mod table;

pub use column::{ColumnWidth, IncludeNeeds, IntoColumns, TextColumn};
pub(crate) use commit::run_after_commit;
pub use commit::{Committed, Mutation};
pub use filter::{DateFilter, Filter, IntoFilters, SelectFilter, TernaryFilter, VariantFilter};
use naming::{kebab_case, pluralize, type_short_name};
pub use navigation::{NavTarget, NavigationItem};
pub use relation::{
    IntoRelationColumns, MAX_RELATION_ROWS, RelationColumn, RelationColumns, render_relation,
};
pub(crate) use state::{
    BULK_DELETE_ROUTE_SEGMENT, CREATE_ROUTE_SEGMENT, DELETE_ROUTE_SEGMENT, EDIT_ROUTE_SEGMENT,
    RECORD_ROUTE_PARAM, create_page_url, cursor_after, cursor_before, cursor_none,
};
pub use state::{Sort, TablePage, TableSignals, TableState};
pub(crate) use table::TableChrome;
pub use table::{GroupDef, GroupKey, OrderMode, RowActions, RowKey, RowPolicy, Table};

#[cfg(test)]
pub(crate) use crate::query_term::MAX_QUERY_TERM;
pub(crate) use crate::query_term::clamp_query_term;

/// Maps one Toasty `Model` to its admin UI.
///
/// # Contract
///
/// **Every method has a default**, so a resource compiles the moment it
/// declares a [`Model`](Self::Model) — and an omission must fail loudly
/// rather than silently:
///
/// - **Checked at [`Panel::build`](crate::panel::Panel::build)**: the table must declare columns
///   and a row key, and with [`can_create`](Self::can_create) the [`form`](Self::form) must declare
///   fields. `table`, `form` and `can_create` are declarations and take no request context, because
///   build checks them with a Db-only context.
/// - **Loud at request time**: the record fns ([`create_record`](Self::create_record),
///   [`update_record`](Self::update_record), [`delete_record`](Self::delete_record)) default to an
///   error naming the type, so a resource that never implemented delete says so instead of writing
///   nothing quietly. [`bulk_delete_records`](Self::bulk_delete_records) loops `delete_record` by
///   default, so it stays loud through the same stub.
/// - **Opt-in chrome, gated per record**: [`deletable`](Self::deletable) and
///   [`editable`](Self::editable) are whole-resource flags, false by default; see
///   [`deletable`](Self::deletable) for how the row predicates narrow them.
/// - **Default-deny is untouched**: every `can_*` defaults to `false`, so an unconfigured resource
///   exposes no data and no mutation.
pub trait Resource: Sized + Send + Sync + 'static {
    /// The persisted model this resource administers.
    ///
    /// `Send + Sync` holds for every data-only model struct and is required
    /// for concurrent rendering of the resource's pages.
    ///
    /// `Clone` is part of the contract because a committed mutation names its
    /// rows: a handler keeps a copy of the rows it loaded while the
    /// record fn consumes them, so the hook can be handed what was written.
    type Model: toasty::schema::Model + Send + Sync + Clone + 'static;

    /// Whether the current user may view the list page.
    ///
    /// Also gates relationship option loads: a related resource
    /// that denies this cannot offer its records as options at all.
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may view the given record.
    ///
    /// Checked on the edit page (GET), the edit POST (which requires both
    /// `can_view` and `can_update`), per row in CSV export, on each
    /// record behind a relationship `Select`'s options, on each row
    /// of a detail page's relation table, and on the list page as
    /// the per-row gate of every action link (GH #235: the View link, and the
    /// `can_view` half of Edit and Delete). Note both hooks default-deny: a
    /// resource used as a relationship target for option loads must allow
    /// `can_view_any` **and** `can_view` (overriding one does not imply the
    /// other), while a relation table consults `can_view` alone. The
    /// list page deliberately checks only
    /// `can_view_any` for *membership*: `can_view` is an in-memory Rust
    /// predicate that cannot run in SQL, and filtering rows after cursor
    /// pagination would mislabel pages. Row-level visibility that must hold on
    /// the list belongs in [`Self::query`].
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
    /// `can_view` — the edit contract: a record that cannot be
    /// viewed cannot be deleted by UUID-guessing the route.
    fn can_delete(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether this resource exposes row and bulk delete chrome.
    ///
    /// Chrome is opt-in: the default renders no Delete button, no bulk bar and
    /// no confirmation dialog, because the server policy
    /// ([`can_view`](Self::can_view) and [`can_delete`](Self::can_delete), both
    /// default-deny) answers 403 to every one of them. Override to `true`
    /// alongside those predicates.
    ///
    /// This flag is the whole-resource gate; the panel wires the predicates into
    /// the table's row policy ([`Table::row_actions`]), so a row they refuse
    /// renders no Delete link and a disabled bulk checkbox. The handler keeps
    /// its all-or-nothing check as the safety net for a hand-crafted POST.
    fn deletable() -> bool {
        false
    }

    /// Whether this resource exposes row edit chrome.
    ///
    /// The same opt-in flag and per-row rule as [`Self::deletable`], gated by
    /// [`can_view`](Self::can_view) + [`can_update`](Self::can_update).
    fn editable() -> bool {
        false
    }

    /// How one record is displayed on the detail page, read-only.
    ///
    /// The same [`Schema`] a form uses, rendered for reading: a `TextInput`
    /// shows its stored value instead of an `<input>`, a `Select` shows the
    /// option label the form offered, and a layout block keeps the structure it
    /// declares. Declaring a view is what turns the detail page on — the default
    /// declares nothing, so the route 404s and no `View` row action renders.
    ///
    /// Values come from [`hydrate_form_values`](Self::hydrate_form_values). A
    /// relation is not one of these fields — it is a list of records, not a
    /// string — and renders through [`view_relations`](Self::view_relations).
    ///
    /// Read-only is a promise, not a disabled form: nothing here validates or
    /// submits, and no field renders a required marker or an error slot.
    fn view(_cx: &Cx) -> crate::schema::Schema {
        crate::schema::Schema::empty()
    }

    /// The related records on this resource's detail page.
    ///
    /// [`view`](Self::view) renders from the record's *string projection* — one
    /// `HashMap<String, String>` — because that is what every field binds. A
    /// relation is not a string and may be a list of records, so it cannot ride
    /// that map; this hook renders it instead, with the loaded record in hand.
    ///
    /// This is the half that makes [`Self::query`]'s `include` pay: the related
    /// rows are already loaded on the record, so a hook that reads
    /// `record.comments.get()` issues no query at all. Touching an un-included
    /// relation panics ([`Deferred::get`](toasty::Deferred)), so check
    /// [`Deferred::is_unloaded`](toasty::Deferred::is_unloaded) — the same marker
    /// the list columns check.
    ///
    /// Returns `None` (the default) for a resource with no related records to
    /// show, which renders nothing. The two lifetimes are deliberately separate:
    /// the returned view may borrow the request context, never the record — a
    /// view holding the record would pin the handler's local binding for as long
    /// as the page, which does not compile, and the projections return owned
    /// strings ([`render_relation`]), so nothing needs to.
    fn view_relations<'a>(
        _cx: &'a Cx,
        _record: &Self::Model,
    ) -> Option<topcoat::view::BoxView<'a>> {
        None
    }

    /// Whether this resource declares a detail page.
    ///
    /// Derived from [`view`](Self::view) rather than declared twice, so the
    /// route and the row link cannot disagree with the schema that renders
    /// them. The detail handler uses it to 404 a resource that declares
    /// nothing, and the row chrome uses it to leave the link off.
    fn viewed(cx: &Cx) -> bool {
        !Self::view(cx).is_empty()
    }

    /// The record's label in the detail page's title, or `None` when
    /// the record has no label to show.
    ///
    /// The detail page titles itself with this label when a resource returns
    /// `Some`, and with [`navigation_label`](Self::navigation_label) plus the
    /// URL's record key when it returns `None` — the default, so a resource
    /// that declares nothing keeps the title it has. The showcase's
    /// `PostResource` is the worked example: it returns the post's title, so
    /// its heading reads the title instead of `Blog Posts <record key>`.
    ///
    /// `cx` is the request's context — the same one [`view`](Self::view) and
    /// [`hydrate_form_values`](Self::hydrate_form_values) receive — so a label
    /// can read request state (a locale, a tenant). The default ignores both
    /// arguments and returns `None`.
    ///
    /// A label is display text, not a key. Two records can share one (two
    /// users named Ada), so it cannot replace [`Table::id`], whose projection
    /// must stay injective within a page for keyed diffs and bulk selection
    /// or [`Table::pk`], which the action routes resolve as the
    /// model's typed PK.
    fn record_label(_cx: &Cx, _record: &Self::Model) -> Option<String> {
        None
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

    /// Base query — the seam for a resource's **own** row scoping (ADR-0002):
    /// soft deletes, row-level visibility, and the relations a page loads.
    ///
    /// **Tenancy is not this method's job.** When
    /// [`requires_tenant`](Self::requires_tenant) is `true` the framework ANDs
    /// the tenant filter, derived from the model's `tenant_id` column, onto
    /// whatever this returns, at every loader through [`scoped_query`]. Do not
    /// re-state `tenant_id().eq(tenant_id(cx))`: the copy is redundant, and one
    /// that disagreed with the derived column would hide rows rather than widen
    /// access. Code outside the framework's loaders starts from
    /// [`scoped_query`].
    ///
    /// Returns the raw typed statement query, which composes generically —
    /// `filter`, `order_by` and `Paginate::new` work on the raw form for any
    /// `M: Model` — so one panel handler drives every resource's list page.
    ///
    /// # Keep unique constraints in step with this scope
    ///
    /// The app-side unique pre-check probes through the same tenant-scoped
    /// query, so a `#[unique]` index *broader* than the scope is invisible: the
    /// probe misses the colliding row and the user gets a 500 instead of the
    /// inline "has already been taken". Scope the constraint to match —
    /// `#[unique(tenant_id, email)]`. Not checkable at declaration time, since
    /// a query's filters are not introspectable; upstream #117 is the fix.
    fn query(_cx: &Cx) -> toasty::stmt::Query<List<Self::Model>> {
        toasty::stmt::Query::<List<Self::Model>>::all()
    }

    /// [`Self::query`] narrowed to the relations `needs` asks for.
    ///
    /// A loader that reads only part of what [`Self::query`] loads states the
    /// includes it reads here, and the resource answers with the matching branch
    /// of its base query. The names are the opaque vocabulary [`IncludeNeeds`]
    /// documents; the resource maps them onto its typed `include(..)` calls.
    ///
    /// **The default ignores `needs` and returns [`Self::query`] unchanged**, so
    /// narrowing is opt-in per resource. An override must keep the non-tenant
    /// scope [`Self::query`] carries and any relation its
    /// [`can_view`](Self::can_view) reads, because the loaders run that
    /// predicate over the loaded rows; the tenant half is the framework's to AND
    /// on. The list and the export ask for their table's
    /// [`include_needs`](Table::include_needs). The detail page reads
    /// [`view_relations`](Self::view_relations), an opaque hook with no
    /// declaration, so it loads [`Self::query`] unchanged; edit, delete, bulk
    /// delete and the option and pagination probes read no relation, so they ask
    /// for an empty set.
    fn query_with(cx: &Cx, _needs: &IncludeNeeds) -> toasty::stmt::Query<List<Self::Model>> {
        Self::query(cx)
    }

    /// The CSV export's base query: [`Self::query`], narrowed to the relations
    /// the rendered columns declared.
    ///
    /// The export writes a cell per column, so it asks the table which relations
    /// those columns' projections read ([`Table::include_needs`] — each column
    /// declares them with [`TextColumn::needs`]) and hands the answer here.
    /// `docs/guide/src/resources.md` states the override contract: one branch per
    /// declared name, so an include `query` carries for the detail page or the
    /// live list rides along on an export only when a rendered column declared
    /// it.
    ///
    /// **The default delegates to [`Self::query_with`]**, which returns
    /// [`Self::query`] unchanged unless the resource overrides it. Over-fetching
    /// costs a join; dropping a relation a column reads breaks the render, so the
    /// default over-fetches.
    ///
    /// # What an override must keep
    ///
    /// - **The non-tenant scope of [`Self::query`]** (soft deletes, row-level visibility): the
    ///   export is a reader like any other, and an override that drops that half exports other
    ///   rows. The tenant half is the framework's.
    /// - **Whatever the policy path reads.** The visibility scan calls
    ///   [`Self::can_view`](Self::can_view) on every row of both passes before any cell is written,
    ///   so a `can_view` that reads a relation needs it included even though no column declared it.
    /// - **Every name a column declared.** A declared name with no matching include renders an
    ///   unloaded relation, which the column's `is_unloaded` guard (ADR-0011) reports in test
    ///   builds instead of a silent `"-"`.
    fn export_query(cx: &Cx, needs: &IncludeNeeds) -> toasty::stmt::Query<List<Self::Model>> {
        Self::query_with(cx, needs)
    }

    /// Whether this resource requires a tenant in every handler.
    ///
    /// Opt-in and default-open: `false` preserves [`Self::query`] exactly as
    /// written. `true` means two things:
    ///
    /// 1. **The gate.** Every handler 403s when the request carries no tenant, instead of leaking
    ///    unscoped rows or minting nil-tenant orphans.
    /// 2. **The scope.** Every loader ANDs `tenant_id = <request tenant>` onto the base query,
    ///    deriving the column from the model's own schema or the resource's [`Self::tenant_scope`].
    ///    A gated resource that declares neither a `tenant_id` UUID column nor an override is
    ///    refused by [`Panel::build`](crate::Panel::build) at boot rather than served unscoped or
    ///    failing per request.
    ///
    /// A resource that must genuinely serve more than the request tenant
    /// declares `false` and scopes in [`Self::query`] by hand, giving up the
    /// gate above along with the derived filter.
    fn requires_tenant() -> bool {
        false
    }

    /// The predicate the framework ANDs onto this resource's base query to
    /// scope it to `tenant`, or `None` when there is nothing to AND.
    ///
    /// The default derives it from the model: `tenant_id = tenant`, on the
    /// field named `tenant_id` whose type is a UUID (see [`crate::tenancy`]).
    /// Override it when the resource's tenancy is not a column on its own
    /// model — a row that inherits its parent's tenant states the relation
    /// path here instead, and the framework applies it exactly as it applies
    /// the derived one. The showcase's comments are the worked example.
    ///
    /// Only consulted when [`requires_tenant`](Self::requires_tenant) is
    /// `true`. `None` from a gated resource is a **misdeclaration**, not a way
    /// to be unscoped: [`Panel::build`](crate::Panel::build) refuses it at boot
    /// and every loader keeps answering an error naming the resource
    /// rather than running its query without a tenant predicate — the backstop
    /// for a predicate that is only `None` for some tenants. There is
    /// deliberately no override that *removes* the scope — a resource that
    /// must serve more than one tenant declares `requires_tenant() = false`
    /// and owns the scope in [`Self::query`], visibly, with the gate given up.
    fn tenant_scope(tenant: uuid::Uuid) -> Option<toasty::stmt::Expr<bool>> {
        crate::tenancy::derived_tenant_filter::<Self::Model>(tenant)
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

    /// The schema the create and edit forms render, and the source of the
    /// fields the panel validates and hydrates.
    ///
    /// The default is empty, so a resource with no form still lists.
    fn form(_cx: &Cx) -> Schema {
        Schema::empty()
    }

    /// Sidebar entry for the resource.
    ///
    /// The default declares a label ([`Self::navigation_label`]) and no URL:
    /// the Panel that owns the resource resolves where it is mounted, so this
    /// entry never links at a mount the resource guessed.
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
    /// transaction: `ex` is the open tx — run every statement
    /// through it (`exec(&mut *ex)`) and never open a second handle, so the
    /// write commits atomically with the handler's checks. The default
    /// implementation returns an error; resources should override to perform
    /// the actual `toasty::create!` (or `Insert`).
    ///
    /// Return the created row — `toasty::create!` already hands it back, and
    /// the framework cannot otherwise name what a create wrote: the primary key
    /// is the database's (or the app's) to generate, so it is only knowable
    /// from the row. That is what [`Self::after_commit`] receives for a create.
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

    /// Update the already-authorized `record` from form values,
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
    /// subscribers with stale values.
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

    /// Delete the already-authorized `record` (#86): same checked-
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

    /// Bulk-delete the already-authorized `records`: the handler
    /// fetches through the tenancy-scoped `IN` query inside the framework
    /// transaction and checks `can_delete` on every row before calling this.
    /// The default deletes each record through [`Self::delete_record`] in
    /// order, through the same `ex` — any error rolls the whole batch back,
    /// so mid-loop failures delete zero rows. Override for a single-statement
    /// batch.
    fn bulk_delete_records(
        cx: &Cx,
        records: Vec<Self::Model>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move {
            for record in records {
                Self::delete_record(cx, record, &mut *ex).await?;
            }
            Ok(())
        }
    }

    /// Post-commit work for a mutation this resource committed.
    ///
    /// The place for a side effect that must not survive a rollback: an email, a
    /// webhook, an audit row, cache invalidation. Called once per successful
    /// write, after `tx.commit()` and before the response — running it inside a
    /// record fn would leak the effect on rollback, and the write handlers' pool
    /// discipline forbids a second handle while the transaction is open.
    ///
    /// [`Committed`] names the mutation and the rows it wrote: the committed row
    /// a create or update returned, the rows a delete or bulk delete removed
    /// (one call with every row). It is never called when nothing committed: a
    /// validation error, a policy denial, a failed record fn, or a failed commit
    /// all leave the hook untouched, so a rollback cannot produce the effect. A
    /// hook that returns `Err` is logged and ignored — the write is committed,
    /// and retries are the app's to build; a panic surfaces as Topcoat's
    /// panic-isolated 500.
    ///
    /// ```ignore
    /// async fn after_commit(cx: &Cx, committed: Committed<Post>) -> Result<()> {
    ///     let mut db = db(cx); // a fresh handle is allowed here
    ///     for post in committed.records() { notify(post).await?; }
    ///     Ok(())
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
    /// `cx` carries the app schema. A scalar projection needs no
    /// request context, but an embedded **value** does: its keys are the
    /// columns the compiled mapping resolves
    /// ([`write_embedded`](crate::schema::write_embedded)), so this override
    /// does not re-derive them. The context is the
    /// request's, the same one `form(cx)` and the record fns receive.
    fn hydrate_form_values(_cx: &Cx, _record: &Self::Model) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// The tenant-scoped full base query.
///
/// [`Resource::query`] with the predicate from [`Resource::tenant_scope`]
/// ANDed onto it, so a resource that overrides `query` for includes or soft
/// deletes cannot drop the tenant scope by forgetting to re-state it. This is
/// the entry point for a reader that wants the full base query — the detail
/// page and app code. A framework loader that reads only part of it uses
/// `scoped_query_with`.
///
/// # Errors
///
/// - A gated resource and no tenant in `cx` → 403, the same fail-closed answer the handler gate
///   gives.
/// - A gated resource that supplies no tenant predicate — no discoverable `tenant_id` UUID column,
///   no [`Resource::tenant_scope`] override → an error naming the resource and the model.
///   [`Panel::build`](crate::Panel::build) refuses that declaration at boot, so this is the
///   backstop for a predicate that is `None` for the request's tenant, and for app code outside a
///   panel. It is deliberately **not** a fallback to the unscoped query: a silent miss would be the
///   leak [`Resource::requires_tenant`] exists to prevent.
///
/// App code that loads rows itself must call this — on a gated resource
/// [`Resource::query`] is the *tenant-unscoped* base by design, so calling it
/// directly is safe only for rows whose tenant membership is already settled (a
/// write by id against a record the framework loaded and authorized).
pub fn scoped_query<R: Resource>(cx: &Cx) -> Result<Query<List<R::Model>>> {
    apply_tenant_scope::<R>(cx, R::query(cx))
}

/// [`scoped_query`] over a loader's declared includes: the same tenant gate
/// and derived predicate, seeded from [`Resource::query_with`] instead of
/// [`Resource::query`]. A loader that reads no relation passes
/// `IncludeNeeds::default()`.
pub(crate) fn scoped_query_with<R: Resource>(
    cx: &Cx,
    needs: &IncludeNeeds,
) -> Result<Query<List<R::Model>>> {
    apply_tenant_scope::<R>(cx, R::query_with(cx, needs))
}

/// AND the framework's tenant predicate onto `query`.
///
/// The body of [`scoped_query`] and [`scoped_query_with`], split out so every
/// seed — the base query, a loader's narrowed branch, the export's
/// [`Resource::export_query`](Resource::export_query) — shares one gate, one
/// predicate, and one fail-closed error, and so the seeds cannot drift apart.
/// It is crate-internal because a caller outside the crate always has a
/// `Resource`, and so always wants one of the `scoped_query*` entry points.
pub(crate) fn apply_tenant_scope<R: Resource>(
    cx: &Cx,
    query: Query<List<R::Model>>,
) -> Result<Query<List<R::Model>>> {
    if !R::requires_tenant() {
        return Ok(query);
    }
    let tenant = crate::tenancy::require_tenant(cx)?;
    let Some(filter) = R::tenant_scope(tenant) else {
        // Fail closed and loudly: the resource declared a gate whose scope the
        // framework cannot derive and the resource did not state, and running
        // the query unscoped is the one outcome that declaration exists to
        // prevent. `Panel::build` already refused the resource if *no* tenant
        // could scope it; this is the backstop for a `tenant_scope`
        // that answers `None` only for this tenant, and for callers outside a
        // panel.
        tracing::error!(
            resource = R::slug(),
            model = std::any::type_name::<R::Model>(),
            "requires_tenant is true but the resource supplies no tenant predicate: no `tenant_id` \
             UUID column on the model and no `tenant_scope` override (GH #223)"
        );
        return Err(std::io::Error::other(format!(
            "resource '{}' requires a tenant, but the framework cannot scope it: {} declares no \
             `tenant_id` UUID column to derive the filter from, and the resource does not override \
             `tenant_scope` (GH #223); declare the column, override `tenant_scope`, or drop \
             `requires_tenant` and scope in `query`",
            R::slug(),
            std::any::type_name::<R::Model>(),
        ))
        .into());
    };
    Ok(query.filter(filter))
}

/// Every `Resource` is an [`OptionSource`](crate::schema::OptionSource) — the
/// bridge that lets the relationship option loaders be generic over the source
/// surface instead of over `Resource`, so `schema` does not depend on
/// `resource`.
///
/// A resource answers the loaders here, so the loaders never name `Resource`.
/// [`scoped_query`](crate::schema::OptionSource::scoped_query), the one required
/// method, forwards to [`scoped_query`], so an option load inherits the tenant
/// gate and the derived tenant predicate exactly as every other loader does; it
/// is deliberately not [`Resource::query`], which on a gated resource is the
/// *tenant-unscoped* base.
///
/// [`options_query`](crate::schema::OptionSource::options_query) forwards to
/// `scoped_query_with` with an empty [`IncludeNeeds`]: an option load projects a
/// value and a label off each row and reads no relation of its own, so a
/// resource that narrows its loaders narrows option loads too. A source that
/// overrides nothing keeps the full [`scoped_query`]. The policy predicates and
/// the tenant declaration forward unchanged, and the search expression and
/// default ordering come from the resource's declared [`table`](Resource::table),
/// which is where "the option search searches the related resource's searchable
/// columns" lives.
impl<R: Resource> crate::schema::OptionSource for R {
    type Model = R::Model;

    fn scoped_query(cx: &Cx) -> Result<Query<List<R::Model>>> {
        scoped_query::<R>(cx)
    }

    fn options_query(cx: &Cx) -> Result<Query<List<R::Model>>> {
        scoped_query_with::<R>(cx, &IncludeNeeds::default())
    }

    fn can_view_any(cx: &Cx) -> bool {
        <R as Resource>::can_view_any(cx)
    }

    fn can_view(cx: &Cx, record: &R::Model) -> bool {
        <R as Resource>::can_view(cx, record)
    }

    fn requires_tenant() -> bool {
        <R as Resource>::requires_tenant()
    }

    fn slug() -> String {
        <R as Resource>::slug()
    }

    fn search_expr(cx: &Cx, term: &str) -> Option<toasty::stmt::Expr<bool>> {
        <R as Resource>::table(cx).search_expr(term)
    }

    fn order_by(cx: &Cx) -> Option<toasty::stmt::OrderByExpr> {
        <R as Resource>::table(cx).order_by(false)
    }
}

#[cfg(test)]
mod tests {
    use toasty::Db;
    use topcoat::context::CxTestBuilder;

    use super::*;
    use crate::test_support::User;

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
        // Proves the seam composes with the `db(cx)` helper without taking
        // ownership of the query.
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

    /// A gated resource over a model the framework cannot scope from: declared
    /// as requiring a tenant, no `tenant_id` column to derive the filter from.
    struct Misdeclared;

    impl Resource for Misdeclared {
        type Model = User;

        fn requires_tenant() -> bool {
            true
        }
    }

    /// the failure mode is an error naming the resource, not a
    /// fallback to the unscoped query. `User` has no `tenant_id`, and
    /// `scoped_query` must refuse to answer rather than serve every row.
    #[test]
    fn gated_resource_without_a_tenant_column_fails_closed() {
        let cx = CxTestBuilder::new()
            .request_context(crate::Tenant(uuid::Uuid::new_v4()))
            .build();
        let error = scoped_query::<Misdeclared>(&cx).expect_err("must not run unscoped");
        let message = error.to_string();
        assert!(
            message.contains("misdeclareds"),
            "the error must name the resource: {message}"
        );
        assert!(
            message.contains("tenant_id") && message.contains("tenant_scope"),
            "the error must name both ways to scope it: {message}"
        );
    }

    /// A gated resource whose tenancy is not a column on its own model declares
    /// the predicate itself — the shape the showcase's comments need,
    /// where the tenant lives on the parent post. `name` stands in for the
    /// relation path here: the point is that the hook is consulted and ANDed.
    struct DeclaredScope;

    impl Resource for DeclaredScope {
        type Model = User;

        fn requires_tenant() -> bool {
            true
        }

        fn tenant_scope(tenant: uuid::Uuid) -> Option<toasty::stmt::Expr<bool>> {
            Some(User::fields().name().eq(tenant.to_string()))
        }
    }

    #[tokio::test]
    async fn declared_tenant_scope_is_anded_onto_the_base_query() {
        let mut db = Db::builder()
            .models(toasty::models!(User))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let mine = uuid::Uuid::new_v4();
        let theirs = uuid::Uuid::new_v4();
        for name in [mine.to_string(), theirs.to_string()] {
            toasty::create!(User { name }).exec(&mut db).await.unwrap();
        }
        let cx = CxTestBuilder::new()
            .app_context(db)
            .request_context(crate::Tenant(mine))
            .build();
        let mut db = crate::db::db(&cx);
        let rows = scoped_query::<DeclaredScope>(&cx)
            .expect("a declared scope is not a misdeclaration")
            .exec(&mut db)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, mine.to_string());

        // And the gate still runs first: no tenant, no query.
        let tenantless = CxTestBuilder::new().build();
        assert!(scoped_query::<DeclaredScope>(&tenantless).is_err());
    }
}
