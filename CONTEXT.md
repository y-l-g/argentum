# Argentum

Admin toolkit for Rust — server-rendered on Topcoat, persisted with Toasty. Provides the CRUD core of Filament (Panel + Resource → Table + Schema + Action) with no Livewire port, single-panel/single-tenant in Phase 1, explicit preloading and cursor pagination, and a narrow reactivity seam that works on today's shard runtime and migrates to signals v2 + boundary streaming.

## Language

### Panel
The admin application. Owns the Router, the Db in app_context, the layout shell, and a closed set of Resources. A deployment has one Panel in Phase 1.

_Avoid_: Admin, Dashboard, App, Site

### Resource
A type that maps one Toasty Model to its admin UI. Defines the base query, the table, the form (and infolist stub), its pages, navigation entry, and policy. One Model → one Resource.

_Avoid_: Model, Entity, Collection, AdminModel, CRUD

### Schema
The unified layout primitive for forms and infolists. A composition of layout blocks (Section, Group, Grid, Tabs, Wizard) and typed fields (TextInput, Select, Toggle) bound via field lenses to a Model.

_Avoid_: Form, Infolist, Fieldset (as top-level term), statePath

### Table
The declarative description of a list view. Declares columns, filters, search, sort, pagination, and row/bulk actions. It also declares how to query — searchable and filterable columns produce Toasty predicates, sortable columns map to order_by. Owns the row loop: row identity is enforced by construction via `key: &row.id`, never left to the user.

_Avoid_: Grid, Listing, DataTable

### Action
A user-invoked operation, optionally with a modal Schema. Runs via a procedure endpoint, inside a transaction, with authorization checked against the passed record inside the handler.

_Avoid_: Command, Mutation, Operation, Modal

### Query
The base filtered query for a Resource. Returned by Resource::query(cx) and used by every loader. The single seam for tenancy and soft-delete scoping.

_Avoid_: Scope, EloquentQuery, Builder (as domain term)

### Policy
The per-Resource authorization rules (viewAny, view, create, update, delete). Default-deny; checked in both page and shard/procedure handlers.

_Avoid_: Guard, Permission, Gate, Ability

### NavigationItem
An entry in the Panel sidebar. Derived by default from a Resource, overridable for manual grouping and ordering.

_Avoid_: MenuItem, NavLink, SidebarEntry

### Column
A typed projection of a Model field (or computed value) displayed in a Table row. Declares rendering and whether it is searchable or sortable, mapping to a Toasty field lens and order_by.

_Avoid_: Field (in table context), Cell, Attribute

### Filter
A predicate contributed to a Table's query. A typed wrapper around a Toasty Expr<bool> produced from a UI control (SelectFilter, TernaryFilter), composed with AND.

_Avoid_: Scope, Constraint, Where

### Field
A typed input bound to a Model lens inside a Schema. Knows its nullability, uniqueness, and column name, and hydrates from the Model into Create/Update projections.

_Avoid_: Input, Control, Widget (in form context), statePath

### Boundary
A Topcoat region whose rendered output is hashed and diffed across renders. Used to ship only the changed Table grid on search/sort/page changes. Every Table is a Boundary by default; deferring the initial load is opt-in.

_Avoid_: Shard (as domain term), Region, Island

### Notification
A transient user-visible message (status + title, ~4s) produced by an Action's result, rendered in a top-level boundary owned by the Panel layout so it survives Table boundary swaps.

_Avoid_: Toast, Flash, Alert (as domain term)

### EmptyState
The Table's zero-rows rendering (icon + title + optional action), shown for "no records" and "no search results".

_Avoid_: NoResults, Placeholder, ZeroState

### ErrorState
The Table's failed-load rendering, produced by the layout's slot: Result match rather than inside the shard, so it survives boundary swaps.

_Avoid_: ErrorPage, Fallback
