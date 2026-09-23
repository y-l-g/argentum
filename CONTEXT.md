# Argentum

Admin toolkit for Rust — server-rendered on Topcoat, persisted with Toasty. Provides the CRUD
core of Filament (Panel + Resource → Table + Schema, deletes via Resource record fns) with no
Livewire port, explicit preloading and cursor pagination, and a narrow reactivity seam: streamed
`suspense` regions render the list shell first and swap the loaded table in, while reruns morph in
place (focus survives) and tables opting into `Table::live_search` re-render their table in place
through the slug-dispatched `table_search` shard: search, sort, filters, and pagination write
signals, and the table morphs without a navigation (ticket #104, GH #151).

> **Shipped vs spec:** everything the terms below call shipped — Panel, Resource, Table, Schema,
> Policy, authentication, tenancy, and uploads — lives in `argentum-core`. `examples/showcase` is
> the runnable reference and `docs/guide` the user guide for what each seam does in detail. A term
> marked spec-level or future work is not implemented.

## Language

### Panel

The admin application. Owns the Router, the Db in app_context, the layout Shell, its declared
Resources, and the default-on authentication gate (ADR-0013). Declaring a Panel with Resources
yields resource routes and navigation; an app's layout delegates to `Panel::layout_shell` for the
Shell with no manual document HTML.

_Avoid_: Admin, Dashboard, App, Site

_Documented exceptions_: shipped `AdminUser` model retains the `Admin` prefix (auth seam,
ADR-0013); the `/admin` mount default is generic English for the URL prefix, not Panel vocabulary.

### Authenticator

The one authentication seam (ADR-0013). An object-safe trait resolving credentials into the erased
`CurrentUser` and a live session back to it; `PasswordAuth` is the shipped default over
`AdminUser`, `Panel::auth(Auth::custom(..))` swaps in an app implementation over its own user
table, and `Auth::disabled()` is the explicit, greppable opt-out. Sessions stay framework-owned
(`AuthSession`) whichever implementation is in use.

_Avoid_: Provider, Guard, LoginManager, AuthDriver

### CurrentUser

The erased identity resolution places in request `Cx`:
`{ id, login, display_name, tenant_id, can_access_panel }`. Pages, shards, and app code read it
only through `current_user(cx)` / `require_authenticated(cx)`; the concrete user model never leaks
past the `Authenticator`. The optional `tenant_id` becomes the request `Tenant`.

_Avoid_: AuthUser, Principal, Account, SessionUser

### Session

A server-side `AuthSession` row keyed by the SHA-256 hash of a client token carried in a hardened
cookie (`__Host-`, HttpOnly, Secure, SameSite=Lax). Seven-day fixed lifetime, rotated on login,
deleted on logout, revocable per user; the raw token is never stored.

_Avoid_: Token (the client half), SessionStore, Login, Cookie

### Resource

A type that maps one Toasty Model to its admin UI. Defines the base query — and the export's
narrowed half of it, `export_query`, which defaults to the base query unchanged (GH #177) — the
tenant declaration (`requires_tenant` gates every handler; `tenant_scope` supplies the predicate
the framework ANDs on, derived from the model's `tenant_id` by default and declared by the
resource when its rows inherit their tenant, GH #223; a gated resource that supplies neither is
refused at `Panel::build`, GH #231), the table, the form, the record's string projection for
edit/view hydration (`hydrate_form_values(cx, record)`), the view (GH #187), navigation entry, and
policy — the `can_*` predicates, which the panel applies per record to the list's action chrome
through the table's row policy (GH #235). One Model → one Resource; its routes
(list/create/view/edit/delete) come from the Panel
registration, not a `pages()` declaration. A resource that declares no `view` has no detail page:
`viewed()` is derived from the schema, not declared beside it, so the route's answer and the row's
`View` link cannot disagree.

_Avoid_: Model, Entity, Collection, AdminModel, CRUD

### Schema

The unified layout primitive for forms, infolists, and detail pages (GH #187, ADR-0016): one
declaration read two ways — `render_with` gives controls, `render_readonly` gives the record's
stored values under the same labels and layout. A composition of layout blocks (Section, Group,
Grid, Tabs) and typed fields (TextInput, Textarea, Select, FileUpload, Repeater) bound via field
lenses to a Model. Textarea is TextInput's multi-line sibling: the same lens, the same required
default and error contract, a `<textarea>` control instead — and deliberately no `unique()`, since
the app-side pre-check builds its probe from `TextInput` (GH #184, GH #115).

_Avoid_: Form, Infolist, Fieldset (as top-level term), statePath

### Table

The declarative description of a list view. Declares columns, filters, search, sort, pagination,
and row/bulk actions. It also declares how to query — searchable and filterable columns produce
Toasty predicates, sortable columns map to order_by. Owns the row loop: row identity is mandatory
and typed, declared once via the table's row-key closure (`Table::id(|u| u.id.to_string())`) until
Toasty exposes instance→PK extraction, and render errors without it — never a loop index. Identity
is two projections: the `Table::id` display key (keyed diffs, DOM ids) and the `Table::pk` record
key (edit/delete URLs, bulk checkbox values), resolved by handlers as the typed PK — action chrome
without `pk` is a render error, not a silent 404 (GH #168).

Row chrome is gated twice. The `with_*` prefixes decide which affordances the table declares at all,
and `TableChrome` is the whole-resource declaration behind them (GH #226); the table's **row
policy** (`Table::row_actions`) decides which of them each loaded record may use (GH #235). The
panel wires the policy from the resource's `can_view`/`can_update`/`can_delete`, each action paired
with the predicates its route checks (GH #86, GH #168), so a refused row renders no Edit/Delete link
and a **disabled** bulk checkbox labelled with the reason. A table that declares no policy renders
every wired action (`RowActions::ALL`), and a table with no chrome never consults one — the
whole-resource gate is unchanged. The handler's all-or-nothing check stays as the safety net for a
hand-crafted POST, which is why a denied row must never reach the selection transport.

A `live_search(true)` table hands its chrome to the page's `TableSignals`: the shard's tracked
reads re-render the table in place when search, sort, filters, or pagination write a signal
(GH #151). Grouping rides the same signal set, seeded from the page-load `?group_by=` and changed
via navigation until a live control exists (GH #157). A page can own the same seam directly —
create the `TableSignals`, render the live toolbar, and let its own shard load through `Table::load`
and re-render with `Table::render_live_with_state` — which is how the showcase table demos stay
live without being resources (GH #154 §2).

_Avoid_: Grid, Listing, DataTable

_Documented exception_: `Grid` is also a shipped **Schema layout block** — `Grid::new(2)`, a column
container for forms and detail pages (ADR-0016). It is a different artifact from the list view, and
only the layout block keeps the name; the rendered list is a Table everywhere, including in code
comments and locals.

### Column

A typed projection of a Model field (or a computed value) displayed in a Table row, rendered
through a lens-bound closure where typos fail at compile time. `searchable`/`sortable` map to
Toasty predicates and order_by; computed columns render values but declare none. A column whose
projection reads a relation declares it with `needs(..)`, and the CSV export's narrowed query is
built from those declarations (GH #177, ADR-0018). A column declares its width in the table's
fixed layout with `width(ColumnWidth::..)`; widths are shares of the table, and the default follows
the column's kind: a field column declares none and takes what the declared columns leave, a
computed column claims a share (GH #240). `TextColumn` is the only column type; Badge,
Number and the rest remain spec-level.

_Avoid_: Field (in table context), Cell, Attribute

### Detail page

`GET {prefix}/{slug}/{id}` (GH #187): one record in two halves — `Resource::view`'s Schema,
read-only, plus `Resource::view_relations(cx, record)` for the related rows the query's `include`
loaded. The Schema renders the record's string projection, so a relation (a list of records) needs
the typed half; loading goes through the tenant-scoped query (`scoped_query`, GH #223) like every
other record page, so an unknown id and one outside the request's scope are the same 404, while a
record the caller may not view is a 403.

_Avoid_: Show page, Infolist page, Record view

### Action

A user-invoked delete/create/edit operation driven by a `Resource` record fn (`delete_record` /
`bulk_delete_records` / `create_record` / `update_record`) through a POST handler, inside a
transaction, with authorization checked against the passed record inside the handler. The four
kinds are the mutation vocabulary, and they exist as one value — `Mutation::Create/Update/Delete` —
which is what a `Committed` carries to `after_commit`. **Not** an operation *type*: a non-CRUD
operation (publish, archive) is still modelled as a record fn or a hand-written page, and an
`Action` value with its own before/after hooks remains future work (GH #112).

_Avoid_: Command, Mutation, Operation, Modal

### Committed

What one successful mutation wrote, handed to `Resource::after_commit` (GH #112): the mutation kind
plus the rows it wrote — the row a create returned, the row an update returned (the committed
state, reloaded by the instance update), the rows a delete or bulk delete removed (gone by the time
the hook sees them, so they arrive as they were). Built by the framework, never by an app. One
`Committed` per write, so a bulk delete is a single value however many rows it took. The hook runs
after `tx.commit()` and before the response, which is the only place a side effect that must not
survive a rollback belongs; a failed hook is logged and never rolls the write back, and a write
that did not commit never produces a `Committed` at all.

_Avoid_: CommittedSet, ChangeSet, Event, PostCommit

### Query

The base filtered query for a Resource. Returned by Resource::query(cx) and used by every loader
*through* `scoped_query(cx)`, which is that base with the framework's tenant filter ANDed on when
the resource requires a tenant (GH #223); the CSV export asks Resource::export_query(cx, needs)
instead, which defaults to this query and may narrow its includes to the ones the exported columns
declared (GH #177), and is scoped the same way. The seam for a resource's own row scoping — soft
deletes, row-level visibility, includes; tenancy is the framework's, so a gated resource's `query`
is its tenant-unscoped base.

_Avoid_: Scope, EloquentQuery, Builder (as domain term)

### Policy

The per-Resource authorization rules (viewAny, view, create, update, delete), implemented as
`Resource::can_view_any`/`can_view`/`can_create`/`can_update`/`can_delete` — the one authorization
vocabulary. Default-deny; checked in both page and POST handlers, and in relationship option loads
(`can_view_any` fails the load closed, `can_view` filters rows before labels render, GH #108). The
row/bulk chrome that promises these actions is opt-in to match (GH #226):
`Resource::editable`/`deletable` default to `false`, so a resource that never declares them renders no
Edit or Delete affordance and its default-deny predicates are never contradicted. A resource that opts
in declares the flag beside the predicate it promises — `can_view` + `can_update` for the Edit link,
`can_view` + `can_delete` for row and bulk Delete — and the panel applies those predicates per row
through the table's row policy (GH #235): a refused row renders no link, and a delete-refused row a
**disabled** bulk checkbox labelled with the reason, so select-all submits only rows the handler
accepts. The
chrome narrows with the rule instead of contradicting it, and the handler's all-or-nothing check
stays as the safety net for a hand-crafted POST. `Resource::viewed` is per-record exact the same
way, derived from the declared `view` schema rather than declared beside it. Nothing enforces the
pairing: `can_update`/`can_delete` need a record, so no `Panel::build` call has one to check, and
Rust cannot tell an overridden method from a defaulted one — a chrome flag beside a row-level
predicate is a legitimate configuration, not a detectable mistake.

_Avoid_: Guard, Permission, Gate, Ability, Policy trait

### editable

A **chrome switch**, not a policy predicate: `Resource::editable()` decides whether the per-row
`Edit` link renders (GH #162). Defaults to `false` — chrome is opt-in (GH #226), matching the
default-deny predicates in `Policy`, so a resource that never declares it renders no Edit
affordance. A resource that opts in overrides it to `true` alongside the predicate it promises
(`can_view` + `can_update` for the edit link), which the panel then applies per record (GH #235):
a row those predicates refuse renders no link. It grants nothing: the edit GET and POST always
require `can_view` + `can_update`, and the routes exist whether or not the link renders. The
coarse flag and a row-level predicate are a legitimate pair — the flag decides whether the column
exists, the predicate decides which rows fill it — while chrome shown for a row the route refuses
is a bug, not a configuration.

_Avoid_: Writable, Mutable, can_edit

### deletable

A **chrome switch**, not a policy predicate: `Resource::deletable()` decides whether the row Delete
button and the bulk checkbox column render (GH #96). Defaults to `false` — chrome is opt-in
(GH #226), matching the default-deny `can_delete`, so a resource that never declares it renders no
Delete affordance. A resource that opts in overrides it to `true` alongside `can_view` +
`can_delete`, which the panel then applies per record (GH #235): a row either predicate refuses
renders no Delete link and a **disabled** bulk checkbox labelled with the reason, so select-all
cannot submit a key the handler's all-or-nothing check refuses. It grants nothing:
`delete_record`/`bulk_delete_records` re-check
`can_delete` on the loaded record inside the handler's transaction, and the routes exist whether or
not the chrome renders.

_Avoid_: Destroyable, Removable, can_delete

### NavigationItem

An entry in the Panel sidebar: a label, a NavTarget, and a sort order. Derived by default from a
Resource, overridable to change the label, the order, or an explicit URL. The Panel that owns the
entry owns the URL: a `Derived` target names none, so the panel resolves it from its own mount
prefix plus the resource's slug, while an explicit URL is a link its author wrote and is kept
verbatim (GH #165).

_Avoid_: MenuItem, NavLink, SidebarEntry

### NavTarget

Where a NavigationItem points. `Derived` means the declaring Resource cannot know its mount, so
the owning Panel resolves the URL; `Url` names a URL outright. The distinction is the type rather
than a convention, so prefix resolution can never touch a URL an author wrote (GH #165).

_Avoid_: Link, Route, Target, SidebarUrl

### Filter

A predicate contributed to a Table's query. A typed wrapper around a Toasty `Expr<bool>` produced
from a UI control (SelectFilter, TernaryFilter, DateFilter, VariantFilter), composed with AND.

_Avoid_: Scope, Constraint, Where

### Field

A typed input bound to a Model lens inside a Schema. A `String` lens binds with `TextInput::r#for`;
a lens whose leaf is another type (`i64`, `Uuid`, `jiff::Timestamp`) binds with `TextInput::typed`
(GH #192), which renders the value's `Display`, parses the submission through the type's own
`FromStr`, and refuses what it cannot parse as an inline field error. The two constructors stay
separate so `r#for`'s "only `String` compiles" rule — the compile-time guarantee of ADR-0001 — is
unchanged at every existing call site. Bound via its field lens and column name; `required` defaults
from Toasty column nullability (GH #100, GH #147 — `TextInput`, `Textarea`, `Select`, `FileUpload`
alike, opt out with `.optional()`; a `FileUpload` suspends its native `required` on an edit, where
the stored path is surfaced instead and an empty control means "keep", GH #184), while uniqueness
metadata is future work (upstream gap #115; non-`TextInput` uniqueness is not declared).
`TextInput::unique()` is the exception, and it implies **presence** (GH #189): the framework stores
`""`, never NULL (GH #89), so a unique field left empty is refused inline as
`"<Label> is required"` — `.optional()` does not lift it, the app-side probe never runs for an
empty value, the rendered control's required marker reads the same predicate, and `Panel::build`
refuses the marker on a column with no unique index. Hydrates from the Model into Create/Update
projections. Renders through the upstream `field` family (topcoat#420): `field` + `field_label`
follow the invalid/disabled state, `aria-invalid` drives the control's destructive border/ring, the
`ac-error` slot carries `role="alert"`, and a `Repeater` renders as `field_set` + `field_legend`. A
`Repeater` group whose inner values are all empty is "absent": its inner `required` inputs do not
fire, and a `required` group yields one label-keyed error (GH #147).

_Avoid_: Input, Control, Widget (in form context), statePath

### Embedded value

A `toasty::Embed` struct or enum stored in the parent row's flattened columns, bound as a **value**
rather than leaf by leaf (GH #191, ADR-0019). `#[derive(EmbeddedForm)]` generates the flat-map ↔
typed conversion, the presence question (`any_present`, behind `submitted`), and a
`form(cx, parent)` of controls; the framework supplies every key from the compiled mapping
(`leaf_key`, `enum_spec`, `write_embedded`, `read_embedded`, `submitted`), so the app declares
neither the columns nor the variant rule. An enum's variant is its **discriminant column**:
hydration writes the stored variant into a visible `Select` over the schema's variant list — each
option submitting the stored value and reading as the variant's name — each variant's payload
renders inside its own marked group, and `variant.js` shows only the chosen one's — so a variant
can be picked on create and changed on edit, while with JavaScript off every group renders, so
nothing the server parses is lost. A read-only page names the stored variant rather than printing
the discriminant. A named discriminant always wins — one the enum does not declare is refused
loudly. Payloads select a variant only when no discriminant is named at all (the create form), by a
variant's own non-shared payload, through resolved keys. Per-field overrides are
`#[form(label = "…")]`, `#[form(textarea)]` and `#[form(textarea, rows = N)]`; an unknown key is a
compile error. A `#[document]` inside a value, a relation, an enum nested inside an enum variant,
and a tuple struct are not covered.

_Avoid_: Nested form, Sub-form, Composite field, Inline model

### Uploader

Where a `FileUpload`'s bytes go (GH #188, ADR-0017): a trait the app implements and installs once
per Panel (`Panel::uploads`), discovered on the app_context wherever a `FileUpload` stores — an
object store is an app-level dependency, not a per-field declaration.
`store(filename, bytes) -> Result<String, String>` receives the part's already-sanitized basename
and its content (bounded by the 10 MiB form cap) and returns the value the record stores, which the
framework renders verbatim as a link to the file (GH #242). A refusal (`Err(reason)`) is an inline
field error — `"<Label> could not be uploaded: <reason>"` — because a rejected upload is user input,
not infrastructure. With no
uploader installed the sanitized basename is stored, and the bytes are drained rather than
buffered. A stored value renders a `clear_<field>` checkbox (a framework transport key, stripped
before any record fn, GH #148); clearing does not waive `required`, so a record that must keep a
file answers `"<Label> is required"` — declare `.optional()` to let a record lose its file.
`Panel::serve_dir(path, dir)` mounts an app-owned filesystem directory (an upload store's output)
on the panel's router, which is the app's only way to add a route the framework does not own. A
served directory is **public** (ADR-0017): those URLs answer whoever asks, with no session, because
the auth gate covers exactly the panel prefix and the runtime prefix (ADR-0013) and a served
directory sits outside both. An app that needs protected files owns that route itself.

_Avoid_: FileStore, Attachment, Media library, Blob store

### Streamed region

A `suspense` region of the page whose content swaps in after the first render. The resource list
streams its table: skeleton first (`Table::render_skeleton`), loaded rows swap in without a client
library. Later reruns (page/shard) morph in place per Topcoat #392 — focus, scroll, and typing
survive; reorderable rows need stable `id`s (ticket #104 and GH #151 cover the live shard:
`Signal<T>` params, #393, written by search/sort/filters/pagination). The table always renders
inside a `data-boundary` region.

_Avoid_: Shard (as domain term), Region, Island, Boundary

### Notification

A transient user-visible message (status + title + optional description, rendered as a
shadcn/Sonner toast, auto-dismissed after ~4s by `notifications.js` with a close button) produced
by a record operation's result, rendered in a shell-level stack owned by the Panel layout so it
survives table swaps. A page can also mount one in place — `notification::live_toast` signals plus
the shell's `live_toaster` shard — so a procedure's result becomes a toast without a navigation
(GH #154 §3).

_Avoid_: Toast (as domain term; the shadcn UI surface is a toast), Flash, Alert

### EmptyState

The Table's zero-rows rendering (icon + title + optional action), shown for "no records" and "no
search results".

_Avoid_: NoResults, Placeholder, ZeroState

### ErrorState

The Table's failed-load rendering: a destructive-accented block (icon + title + optional detail +
retry action) shown **inside** the streamed region when the load `Err`s — the load catches its own
error so the page shell survives and the body is not truncated (GH #79). Distinct from EmptyState:
zero rows is a result, a failed load is not.

_Avoid_: ErrorPage, Fallback

### Shell

The top-level layout that frames every admin page. Owns the Sidebar, topbar, and main content area.

_Avoid_: Layout, Wrapper, Chrome

### Sidebar

The persistent navigation region inside the Shell. The upstream Topcoat `sidebar` primitive (synced
into `argentum-ui`, topcoat#419): header, content, footer, groups and menus, collapsing to
offcanvas on desktop and to its own sheet drawer below `md`. Its open state is runtime signals —
`Panel::render_shell` seeds `open` from the `sidebar_state` cookie, the trigger pair carries
`@click` handlers, and `assets/sidebar.js` mirrors changes back to the cookie.

_Avoid_: Nav, Menu, Drawer

### Page

The standard container for an admin page. Owns max-width, padding and vertical rhythm so pages
declare title and content, not Tailwind layout classes.

_Avoid_: Container, Wrapper, Layout

### Theme

The named set of design tokens that determines the admin's look. Argentum provides **no stylesheet**:
the tokens are the app's, declared in its `styles.css` as the per-app contract of ADR-0006, and
`examples/showcase/styles.css` is the reference — a neutral set that re-tunes upstream's
`--primary`/`--ring`. The one theme component `argentum-ui` owns is `theme_init_script`, a free
function (not a `Theme` type) that reconciles the `dark` class before first paint;
`Panel::dark_mode` supplies its fallback for a visitor with no stored choice.

_Avoid_: Skin, Style, Palette

### Token

A CSS variable (such as `--background`, `--primary`, `--border`) that components reference instead
of raw colors, swapping between light and dark values.

_Avoid_: Variable, Color

### Primitive

A re-exported Topcoat UI component (button, card, badge, table, input...) vendored verbatim from
`topcoat-ui-registry` into `argentum-ui/src/components/primitives/` and synced via
`cargo xtask sync-topcoat-ui`.

_Avoid_: Component (when meaning synced primitive), Widget

### Component

An owned Topcoat `#[component]` in `argentum-ui/src/components/composites/` (Page, ErrorState,
Theme, Toast) that composes Primitives and Tokens. Hand-written, never overwritten by sync.

_Avoid_: Primitive, Widget, Element, View
