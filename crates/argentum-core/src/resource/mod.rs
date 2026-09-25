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
#[cfg(test)]
pub(crate) use state::{filters_param_encodes, reset_filters_param_encodes};
pub(crate) use table::TableChrome;
pub use table::{GroupDef, GroupKey, OrderMode, RowActions, RowKey, RowPolicy, Table};

#[cfg(test)]
pub(crate) use crate::query_term::MAX_QUERY_TERM;
pub(crate) use crate::query_term::clamp_query_term;

/// Maps one Toasty `Model` to its admin UI.
///
/// # Contract (GH #138)
///
/// **Every method has a default**, so a resource compiles the moment it
/// declares a [`Model`](Self::Model) — and an omission must therefore fail
/// loudly rather than silently:
///
/// - **Checked at [`Panel::build`](crate::panel::Panel::build)**, which returns `Err` naming the
///   type: the table must be renderable ([`table`](Self::table) declares columns and a row key)
///   and, where [`can_create`](Self::can_create) allows it, the [`form`](Self::form) must declare
///   fields. `table`, `form` and `can_create` are declarations: they must not need request-scoped
///   context, because build checks them with a Db-only context, and each list and form request
///   calls `table` / `form` again.
/// - **Loud at request time**: the record fns ([`create_record`](Self::create_record),
///   [`update_record`](Self::update_record), [`delete_record`](Self::delete_record),
///   [`bulk_delete_records`](Self::bulk_delete_records)) default to an error naming the type, so a
///   resource that never implemented delete answers "delete not implemented for …" instead of
///   writing nothing quietly.
/// - **Opt-in chrome, gated per record**: [`deletable`](Self::deletable) and
///   [`editable`](Self::editable) default to `false`, so a resource that never mentions them
///   renders no Edit or Delete affordance and cannot advertise an action its default-deny predicate
///   refuses. A resource that wants the chrome declares the flag *and* the matching policy
///   predicates (`can_view` + `can_delete` for `deletable`, `can_view` + `can_update` for
///   `editable`). The flag is the whole-resource gate; the predicates are applied per row, because
///   the panel wires them into the table's row policy ([`Table::row_actions`], GH #235): a row
///   `can_update` refuses renders no Edit link, and a row `can_delete` refuses renders no Delete
///   link and a disabled bulk checkbox. A row-level rule therefore narrows the chrome instead of
///   leaving a control the route answers 403. [`viewed`](Self::viewed) is per-record exact the same
///   way, derived from the declared [`view`](Self::view) schema rather than declared beside it.
/// - **Default-deny is untouched**: every `can_*` still defaults to `false`, so an unconfigured
///   resource exposes no data and no mutation.
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
    /// `can_view` and `can_update`, GH #86), per row in CSV export, on each
    /// record behind a relationship `Select`'s options (GH #108), on each row
    /// of a detail page's relation table (GH #296), and on the list page as
    /// the per-row gate of every action link (GH #235: the View link, and the
    /// `can_view` half of Edit and Delete). Note both hooks default-deny: a
    /// resource used as a relationship target for option loads must allow
    /// `can_view_any` **and** `can_view` (overriding one does not imply the
    /// other), while a relation table consults `can_view` alone (GH #296). The
    /// list page deliberately checks only
    /// `can_view_any` for *membership* (GH #86): `can_view` is an in-memory
    /// Rust predicate that cannot run in SQL, and filtering rows after cursor
    /// pagination would mislabel pages (holes, wrong Next/Prev). Row-level
    /// visibility that must hold on the list belongs in [`Self::query`]
    /// (ADR-0002), which every loader — list, edit, delete, bulk, export —
    /// funnels through *inside* [`scoped_query`], so the tenant half of the
    /// scope is applied after the override rather than by it (GH #223).
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
    /// Chrome is opt-in (GH #226): the default renders no Delete button, no
    /// bulk bar and no confirmation dialog, because server policy
    /// ([`can_view`](Self::can_view) +
    /// [`can_delete`](Self::can_delete), both default-deny) would answer 403 to
    /// every one of them. Override to `true` alongside those predicates.
    ///
    /// This flag is the whole-resource gate; the predicates are applied per row
    /// (GH #235). The panel wires them into the table's row policy, so a row
    /// they refuse renders no Delete link and a disabled bulk checkbox — the
    /// affordance narrows with the rule instead of leaving a control the POST
    /// answers 403 to. The handler keeps its all-or-nothing check as the safety
    /// net for a hand-crafted POST.
    fn deletable() -> bool {
        false
    }

    /// Whether this resource exposes row edit chrome (GH #162).
    ///
    /// Chrome is opt-in (GH #226): the default renders no `Edit` link per row,
    /// because server policy ([`can_view`](Self::can_view) +
    /// [`can_update`](Self::can_update), both default-deny) would answer 403 to
    /// the edit GET. Override to `true` alongside those predicates.
    ///
    /// The same per-row rule as [`Self::deletable`] applies (GH #235): the
    /// panel wires `can_view` + `can_update` into the table's row policy, so a
    /// row the caller may not edit renders no link, matching the edit GET.
    fn editable() -> bool {
        false
    }

    /// How one record is displayed on the detail page (GH #187), read-only.
    ///
    /// The same [`Schema`] a form uses, rendered for
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

    /// The record's label in the detail page's title (GH #241), or `None` when
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
    /// (GH #96), or [`Table::pk`], which the action routes resolve as the
    /// model's typed PK (GH #168).
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

    /// Base query — the seam for a resource's **own** row scoping (ADR-0002 and
    /// its 2026-09-22 status note): soft deletes, row-level visibility, and the
    /// relations a page loads.
    ///
    /// **Tenancy is not this method's job (GH #223).** When
    /// [`requires_tenant`](Self::requires_tenant) is `true` the framework ANDs
    /// the tenant filter — derived from the model's `tenant_id` column — onto
    /// whatever this returns, at every loader, through [`scoped_query`] or the
    /// loader's narrowed `scoped_query_with`. Do not re-state
    /// `tenant_id().eq(tenant_id(cx))` here: the copy is redundant,
    /// and one that disagreed with the derived column would hide rows rather
    /// than widen access. The gate that makes a missing tenant a 403 is
    /// unchanged (GH #87).
    ///
    /// An override of this method is **not** the tenant seam: code outside the
    /// framework's loaders must start from [`scoped_query`], because on a gated
    /// resource this is the *tenant-unscoped* base. See [`scoped_query`] for
    /// why.
    ///
    /// Returns the raw typed statement query:
    /// raw queries compose generically — `filter`, `order_by`, and
    /// `Paginate::new` are available on the raw form for any `M: Model` —
    /// which is what lets [`crate::panel::Panel`] drive every resource's list
    /// page through one handler. Scoping it via `Model::filter(..)` in an
    /// override is enough; the wrapper's extra methods are only needed by
    /// hand-written loaders.
    ///
    /// # Keep unique constraints in step with this scope (GH #88)
    ///
    /// The app-side unique pre-check probes submitted values **through the
    /// tenant-scoped query** — `scoped_query_with` with an empty include set,
    /// the same scope [`scoped_query`] applies (GH #298) — so it only sees the
    /// rows that query returns. A `#[unique]` index *broader* than the scope is
    /// therefore invisible to it:
    /// the probe misses the colliding row, the database refuses the write, and
    /// the user gets a 500 instead of the inline "has already been taken".
    ///
    /// The tenant case is the one that bites — the framework scopes to
    /// `tenant_id` while the column carries a plain `#[unique]` (global), which
    /// makes two tenants sharing a value a legitimate pair to the probe and a
    /// constraint violation to the database. Scope the constraint to match:
    /// `#[unique(tenant_id, email)]`, which also makes it say what it means.
    /// `Author.email` in the showcase is the worked example.
    ///
    /// The invariant cannot be *checked* at declaration time — a query's filters
    /// are not introspectable, so nothing can compare the two automatically.
    /// Upstream #117 (a driver-level unique-violation predicate) is what would
    /// make a mismatch safe rather than merely documented.
    fn query(_cx: &Cx) -> toasty::stmt::Query<List<Self::Model>> {
        toasty::stmt::Query::<List<Self::Model>>::all()
    }

    /// [`Self::query`] narrowed to the relations `needs` asks for (GH #298).
    ///
    /// A loader that reads only part of what [`Self::query`] loads states the
    /// includes it reads here, and the resource answers with the matching
    /// branch of its base query. The names are the opaque vocabulary
    /// [`IncludeNeeds`] documents; the resource maps them onto its typed
    /// `include(..)` calls, exactly as it does for
    /// [`export_query`](Self::export_query).
    ///
    /// **The default ignores `needs` and returns [`Self::query`] unchanged**, so
    /// a resource that overrides nothing keeps its current query at every
    /// loader; narrowing is opt-in per resource, the same safe default
    /// [`export_query`](Self::export_query) takes. A resource that overrides it
    /// must keep the non-tenant scope [`Self::query`] carries (soft deletes,
    /// row-level visibility) on every branch — the tenant half is the
    /// framework's to AND on, see `scoped_query_with` — and must keep any
    /// relation its [`can_view`](Self::can_view) reads, because the loaders run
    /// that predicate over the loaded rows.
    ///
    /// Which loaders ask for what:
    ///
    /// - The **list** asks for its table's columns' [`include_needs`](Table::include_needs), and
    ///   the **export** for the same set through [`export_query`](Self::export_query).
    /// - The **detail page** reads [`view_relations`](Self::view_relations), whose projection is an
    ///   opaque hook with no declaration, so it loads [`Self::query`] unchanged.
    /// - The **edit page, delete, bulk delete, the unique-value probe, the relationship option
    ///   lists and their targeted FK existence check, and the pagination probes** read no relation,
    ///   so they ask for an empty set.
    fn query_with(cx: &Cx, _needs: &IncludeNeeds) -> toasty::stmt::Query<List<Self::Model>> {
        Self::query(cx)
    }

    /// The CSV export's base query: [`Self::query`], narrowed to the relations
    /// the rendered columns declared (GH #177).
    ///
    /// The export writes a cell per column, so it asks the table which
    /// relations those columns' projections read ([`Table::include_needs`] —
    /// each column declares them with [`TextColumn::needs`]) and hands the
    /// answer here. `docs/guide/src/resources.md` states the override contract:
    /// a resource splits its base query in two, one branch per declared name,
    /// so an include `query` carries for the detail page or the live list rides
    /// along on an export only when a rendered column declared it.
    ///
    /// **The default delegates to [`Self::query_with`]**, which in turn returns
    /// [`Self::query`] unchanged unless the resource overrides it. Over-fetching
    /// a relation nothing reads costs a join; dropping one a column does read
    /// breaks the render — the default takes the safe side. A resource that
    /// overrides [`Self::query_with`] therefore narrows its export along with
    /// its other loaders, and overrides this method only when the export needs
    /// a different branch.
    ///
    /// # What an override must keep
    ///
    /// - **The non-tenant scope of [`Self::query`].** This is the same soft-delete/row-level seam
    ///   (ADR-0002), and the export is a reader like any other: an override that drops that half
    ///   exports other rows. The *tenant* half is not the override's to keep — the framework ANDs
    ///   it onto what this returns, exactly as it does for [`Self::query`] (GH #223), so a gated
    ///   export is scoped whether or not the override re-states the filter.
    /// - **Whatever the policy path reads.** The export's visibility scan calls [`Self::can_view`]
    ///   on every row of both passes, before any cell is written, so a `can_view` that reads a
    ///   relation needs that relation included even though no column declared it — include it
    ///   unconditionally in the narrowed branch. Reading an un-included relation panics in
    ///   `Deferred::get`.
    /// - **Every name a column declared.** A declared name with no matching include renders an
    ///   unloaded relation, which the column's `is_unloaded` guard (ADR-0011) reports in test
    ///   builds instead of a silent `"-"`.
    fn export_query(cx: &Cx, needs: &IncludeNeeds) -> toasty::stmt::Query<List<Self::Model>> {
        Self::query_with(cx, needs)
    }

    /// Whether this resource requires a tenant in every handler (GH #87).
    ///
    /// Opt-in and default-open: `false` preserves the default behavior — no
    /// gate, and [`Self::query`] exactly as written. Override to `true` on a
    /// resource whose model carries rows per tenant.
    ///
    /// `true` means two things, and the second is GH #223:
    ///
    /// 1. **The gate.** Every handler 403s when the request carries no tenant, instead of leaking
    ///    unscoped rows or minting nil-tenant orphans (#87's tenantless-create rejection,
    ///    unchanged).
    /// 2. **The scope.** Every loader ANDs `tenant_id = <request tenant>` onto the resource's base
    ///    query, deriving the column from the model's own schema — see [`scoped_query`] and
    ///    [`Self::tenant_scope`]. A gated resource is therefore never unscoped because an override
    ///    forgot to re-state the filter, and it is never unscoped because the derivation failed
    ///    either: the model must declare a `tenant_id` UUID column, or the resource must declare
    ///    its own predicate in [`Self::tenant_scope`], and a gated resource that does neither is
    ///    refused by [`Panel::build`](crate::Panel::build) at boot (GH #231) — the declaration is
    ///    checkable without a request — rather than serving rows unscoped or failing per request.
    ///
    /// A resource that must genuinely serve more than the request tenant — a
    /// deliberate cross-tenant view — declares `false` and scopes in
    /// [`Self::query`] by hand. That is the explicit, visible way out, and it
    /// gives up the gate above along with the derived filter.
    fn requires_tenant() -> bool {
        false
    }

    /// The predicate the framework ANDs onto this resource's base query to
    /// scope it to `tenant` (GH #223), or `None` when there is nothing to AND.
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
    /// (GH #231), and every loader keeps answering an error naming the resource
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
    /// ([`write_embedded`](crate::schema::write_embedded)), so this override
    /// does not re-derive them (GH #185). The context is the
    /// request's, the same one `form(cx)` and the record fns receive.
    fn hydrate_form_values(_cx: &Cx, _record: &Self::Model) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// The tenant-scoped full base query (GH #223).
///
/// [`Resource::query`] with the tenant predicate from
/// [`Resource::tenant_scope`] ANDed onto it: for a resource whose
/// [`requires_tenant`](Resource::requires_tenant) is `true`, the rows are
/// narrowed to the request tenant — derived from `tenant_id` by default,
/// declared by the resource when its tenancy is inherited. The filter is
/// applied *here*, outside the resource's `query`, so a resource that
/// overrides `query` for includes or soft deletes cannot drop the tenant scope
/// by forgetting to re-state it.
///
/// This is the entry point for a reader that wants the full base query — the
/// detail page and app code. A framework loader that reads only part of it uses
/// `scoped_query_with`, which composes the same gate and predicate over
/// [`Resource::query_with`] (GH #298).
///
/// # Errors
///
/// - A gated resource and no tenant in `cx` → 403, the same fail-closed answer the handler gate
///   gives (GH #87).
/// - A gated resource that supplies no tenant predicate — no discoverable `tenant_id` UUID column,
///   no [`Resource::tenant_scope`] override → an error naming the resource and the model.
///   [`Panel::build`](crate::Panel::build) refuses that declaration at boot (GH #231), so this is
///   the backstop for a predicate that exists but is `None` for the request's tenant, and for app
///   code that calls this outside a panel. It is deliberately **not** a fallback to the unscoped
///   query: discovery is by name, and a silent miss would be exactly the leak
///   [`Resource::requires_tenant`] exists to prevent.
///
/// # When to call this
///
/// App code that loads rows itself must — a record fn double-checking a foreign
/// key, a custom page, a test. On a gated resource [`Resource::query`] is the
/// *tenant-unscoped* base by design, so calling it directly is safe only for
/// rows whose tenant membership is already settled (a write by id against a
/// record the framework loaded and authorized).
pub fn scoped_query<R: Resource>(cx: &Cx) -> Result<Query<List<R::Model>>> {
    apply_tenant_scope::<R>(cx, R::query(cx))
}

/// [`scoped_query`] over a loader's declared includes (GH #298).
///
/// The same tenant gate and derived predicate as [`scoped_query`], seeded from
/// [`Resource::query_with`] instead of [`Resource::query`], so a loader that
/// reads only part of the base query's relations pays only for those. The
/// tenant half is applied here rather than by the resource's override, so a
/// narrowed branch cannot drop the scope by forgetting to re-state it.
///
/// A loader that reads no relation passes `IncludeNeeds::default()`.
pub(crate) fn scoped_query_with<R: Resource>(
    cx: &Cx,
    needs: &IncludeNeeds,
) -> Result<Query<List<R::Model>>> {
    apply_tenant_scope::<R>(cx, R::query_with(cx, needs))
}

/// AND the framework's tenant predicate onto `query` (GH #223).
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
        // could scope it (GH #231); this is the backstop for a `tenant_scope`
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
/// `resource` (GH #208).
///
/// A resource answers the loaders here, so the loaders never name `Resource`:
///
/// - [`scoped_query`](crate::schema::OptionSource::scoped_query) — the one required method —
///   forwards to [`scoped_query`], so an option load inherits the tenant gate and the framework's
///   derived tenant predicate exactly as every other loader does (GH #223). It is deliberately not
///   [`Resource::query`], which on a gated resource is the *tenant-unscoped* base.
/// - [`options_query`](crate::schema::OptionSource::options_query) forwards to `scoped_query_with`
///   with an empty [`IncludeNeeds`]: an option load projects a value and a label off each row and
///   reads no relation of its own, so a resource that narrows its loaders (GH #298) narrows option
///   loads too. A source that overrides nothing keeps the full [`scoped_query`].
/// - the policy predicates and the tenant declaration forward unchanged.
/// - the search expression and the default ordering come from the resource's declared
///   [`table`](Resource::table), which is where "the option search searches the related resource's
///   searchable columns" lives (GH #150).
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

    /// GH #223: the failure mode is an error naming the resource, not a
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
    /// the predicate itself (GH #223) — the shape the showcase's comments need,
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
