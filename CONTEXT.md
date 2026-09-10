# Argentum

Admin toolkit for Rust — server-rendered on Topcoat, persisted with Toasty. Provides the CRUD core of Filament (Panel + Resource → Table + Schema, deletes via Resource record fns) with no Livewire port, explicit preloading and cursor pagination, and a narrow reactivity seam: streamed `suspense` regions ship the list shell first and swap the loaded grid in, while reruns morph in place (focus survives) and tables opting into `Table::live_search` re-render their grid keystroke-live through the slug-dispatched `table_search` shard (ticket #104).

> **Shipped vs spec:** every term below is vocabulary-level truth. As of Phase 2 (Relations & polish, spec #63, tickets #64–#71, ADR-0011/0012) plus post-Phase-2 polish (multipart `FileUpload`, single-entry `Repeater`, `VariantFilter`, honest bulk/filter chrome, in-region `ErrorState`) — **Panel** (with opt-in `Brand` + `DarkMode`, `Router`/`Db`/`Shell` + `Resource` routes), **Resource** (`query` tenancy seam + `include` + `TextColumn::computed`/`Select::relationship`), **Table** (`data-boundary` wrapper + streamed `suspense` grid, `searchable`/`sortable`, cursor pagination, `SelectFilter`/`TernaryFilter`/`DateFilter`/`VariantFilter` + `?filters=`, `group_by`/`count` + `to_csv` export, checkbox bulk column), **Schema** (`Section`/`Group`/`Grid`/`Tabs`/`Wizard` + `TextInput`/`Select`/`FileUpload`/`Repeater` + `required`/`email`/`unique` + `relationship`), **deletes** (per-row + bulk via `Resource` record fns, policy-checked), **Policy** (`can_*`, default-deny), **Notification**, **Navigation**, **Query**, **Tenancy** (`Tenant` via `Cx::with` + `tenant_id(cx)`), **Filter**, **Field** ship in `argentum-core` with showcase at `/admin/users` + `/admin/authors` + `/admin/posts` (search/sort/paginate/create/edit/delete/bulk-delete, filters/grouping/export, all policy-checked, notification, tenancy, `benchmarks/` Phase-2 budget 50 rows 2 includes `<40ms p50`). **Page, Theme/Token beyond brand/dark_mode, ChartWidget/StatsOverview, via many-to-many, GROUP BY aggregates** remain future work (tracking issue #38 is closed; see README §10).

## Language

### Panel
The admin application. Owns the Router, the Db in app_context, the layout Shell, and its declared Resources. Declaring a Panel with Resources yields resource routes and navigation; an app's layout delegates to `Panel::layout_shell` for the Shell with no manual document HTML.

_Avoid_: Admin, Dashboard, App, Site

### Resource
A type that maps one Toasty Model to its admin UI. Defines the base query, the table, the form (and infolist stub), its pages, navigation entry, and policy. One Model → one Resource.

_Avoid_: Model, Entity, Collection, AdminModel, CRUD

### Schema
The unified layout primitive for forms and infolists. A composition of layout blocks (Section, Group, Grid, Tabs, Wizard) and typed fields (TextInput, Select, FileUpload, Repeater) bound via field lenses to a Model.

_Avoid_: Form, Infolist, Fieldset (as top-level term), statePath

### Table
The declarative description of a list view. Declares columns, filters, search, sort, pagination, and row/bulk actions. It also declares how to query — searchable and filterable columns produce Toasty predicates, sortable columns map to order_by. Owns the row loop: row identity is mandatory and typed, declared once via the table's row-key closure (`Table::id(|u| u.id.to_string())`) until Toasty exposes instance→PK extraction, and render errors without it — never a loop index.

_Avoid_: Grid, Listing, DataTable

### Column
A typed projection of a Model field (or a computed value) displayed in a Table row, rendered through a lens-bound closure where typos fail at compile time. `searchable`/`sortable` map to Toasty predicates and order_by; computed columns render values but declare none. Badge, Number and other variants remain spec-level.

_Avoid_: Field (in table context), Cell, Attribute

### Action
A user-invoked delete/create/edit operation driven by a `Resource` record fn (`delete_record` / `bulk_delete_records` / `create_record` / `update_record`) through a POST handler, inside a transaction, with authorization checked against the passed record inside the handler.

_Avoid_: Command, Mutation, Operation, Modal

### Query
The base filtered query for a Resource. Returned by Resource::query(cx) and used by every loader. The single seam for tenancy and soft-delete scoping.

_Avoid_: Scope, EloquentQuery, Builder (as domain term)

### Policy
The per-Resource authorization rules (viewAny, view, create, update, delete). Default-deny; checked in both page and POST handlers.

_Avoid_: Guard, Permission, Gate, Ability

### NavigationItem
An entry in the Panel sidebar. Derived by default from a Resource, overridable for manual grouping and ordering.

_Avoid_: MenuItem, NavLink, SidebarEntry

### Filter
A predicate contributed to a Table's query. A typed wrapper around a Toasty Expr<bool> produced from a UI control (SelectFilter, TernaryFilter, DateFilter, VariantFilter), composed with AND.

_Avoid_: Scope, Constraint, Where

### Field
A typed input bound to a Model lens inside a Schema. Bound via its field lens and column name; `required` defaults from Toasty column nullability (GH #100, opt out with `.optional()`), while uniqueness metadata and non-TextInput defaults are future work (see EXTERNAL_GAPS.md). Hydrates from the Model into Create/Update projections.

_Avoid_: Input, Control, Widget (in form context), statePath

### Streamed region
A `suspense` region of the page whose content swaps in after the first render. The resource list streams its grid: skeleton first (`Table::render_skeleton`), loaded rows swap in without a client library. Later reruns (page/shard) morph in place per Topcoat #392 — focus, scroll, and typing survive; reorderable rows need stable `id`s (ticket #104 covers the keystroke-live shard via `Signal<T>` params, #393). `Table::boundary(true)`/`defer(true)` remain as the eager-render demo hooks.

_Avoid_: Shard (as domain term), Region, Island, Boundary (pre-#373 topcoat component, removed upstream)

### Notification
A transient user-visible message (status + title, auto-dismissed after ~4s by `notifications.js` with manual dismiss) produced by a record operation's result, rendered in a shell-level stack owned by the Panel layout so it survives table swaps.

_Avoid_: Toast, Flash, Alert (as domain term)

### EmptyState
The Table's zero-rows rendering (icon + title + optional action), shown for "no records" and "no search results".

_Avoid_: NoResults, Placeholder, ZeroState

### ErrorState
The Table's failed-load rendering: a destructive-accented block (icon + title + optional detail + retry action) shown **inside** the streamed region when the load `Err`s — the load catches its own error so the page shell survives and the body is not truncated (GH #79). Distinct from EmptyState: zero rows is a result, a failed load is not.

_Avoid_: ErrorPage, Fallback

### Shell
The top-level layout that frames every admin page. Owns the Sidebar, topbar, and main content area.

_Avoid_: Layout, Wrapper, Chrome

### Sidebar
The persistent navigation region inside the Shell. Composes header, content, footer, groups and menus, collapsing to an icon rail or sheet drawer on small viewports.

_Avoid_: Nav, Menu, Drawer

### Page
The standard container for an admin page. Owns max-width, padding and vertical rhythm so pages declare title and content, not Tailwind layout classes.

_Avoid_: Container, Wrapper, Layout

### Theme
The named set of design tokens that determines the admin's look. Argentum ships the neutral theme.

_Avoid_: Skin, Style, Palette

### Token
A CSS variable (such as `--background`, `--primary`, `--border`) that components reference instead of raw colors, swapping between light and dark values.

_Avoid_: Variable, Color

### Primitive
A re-exported Topcoat UI component (button, card, badge, table, input...) vendored verbatim from `topcoat-ui-registry` into `argentum-ui/src/components/primitives/` and synced via `cargo xtask sync-topcoat-ui`.

_Avoid_: Component (when meaning synced primitive), Widget

### Component
An owned Topcoat `#[component]` in `argentum-ui/src/components/composites/` (Sidebar, Page, CodeBlock) that composes Primitives and Tokens. Hand-written, never overwritten by sync.

_Avoid_: Primitive, Widget, Element, View
