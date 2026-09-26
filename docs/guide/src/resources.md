# Resources

The `Resource` trait: how one Toasty model maps to its admin UI, what the framework checks at boot,
and how tenancy is declared.

One resource maps one Toasty model to its admin UI:

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Model: toasty::schema::Model + Send + Sync + 'static;
    fn query(_cx: &Cx) -> Query<List<Self::Model>>; // default: Query::all()
    fn query_with(_cx: &Cx, _needs: &IncludeNeeds)
        -> Query<List<Self::Model>>;                // default: query(cx), unchanged
    fn hydrate_form_values(_cx: &Cx, _record: &Self::Model)
        -> HashMap<String, String>;                 // default: empty
    fn export_query(_cx: &Cx, _needs: &IncludeNeeds)
        -> Query<List<Self::Model>>;                // default: query_with(cx, needs)
    fn table(_cx: &Cx) -> Table<Self::Model>;       // default: Table::new(), empty until columns + id
    fn form(_cx: &Cx) -> Schema;                    // default: Schema::empty()
    // plus can_* policy fns (default deny), slug/navigation/requires_tenant
    // defaults, and create/update/delete record fns
}
```

## The contract

Every method is defaulted, so a resource compiles as soon as it names its model — which means an
omission has to fail loudly instead of quietly:

- **At `Panel::build`** (which returns `Result<Router>`): the table must be renderable — `table()`
  declares columns and a row key, plus `Table::pk` where the resource declares action chrome — and
  where `can_create` allows it, `form()` must declare fields. A resource that overrides nothing fails
  the build, naming the type, instead of serving an error state or an empty form. `table()`, `form()`
  and `can_create()` are declarations: `Panel::build` calls them with a Db-only context to check them,
  and each list and form request calls `table()` / `form()` again, so a declaration must not need
  request-scoped context.
- **At request time, loudly**: the record fns default to an error naming the type ("delete not
  implemented for …"), so a missing implementation never looks like a successful no-op.
- **Chrome is opt-in, gated per record**: `deletable()` and `editable()` default to `false`, so a
  resource that never mentions them renders no Edit or Delete affordance — the routes still exist,
  and the default-deny `can_*` predicates answer them. A resource that wants the chrome declares the
  flag **and** the policy predicate it promises: `can_view()` + `can_delete()` for `deletable()`,
  `can_view()` + `can_update()` for `editable()`. It also commits the table to `Table::pk(..)` — the
  action URLs and bulk values carry that projection — and `Panel::build` refuses a table that declares
  chrome without it. The flag is the whole-resource gate (GH #226); the
  predicates are
  applied **per row** (GH #235). The panel wires them into the table's row policy, so a row
  `can_update()` refuses renders no Edit link, a row `can_delete()` refuses renders no Delete link
  and no bulk checkbox, and a row `can_view()` refuses renders
  no View link. Select-all therefore submits only the rows the handler will accept — the showcase's
  SSO-guarded user is the worked example: its row renders no action link and no checkbox. The handler
  keeps its all-or-nothing check on the POST as the safety net for a hand-crafted request. The
  `View` link needs no flag at all — it is derived from whether the resource declares a `view()`
  schema, so there the route and the row link cannot disagree.
- **Default-deny stands**: every `can_*` defaults to `false`, so an unconfigured resource exposes
  no data and no mutation.

## What to know

- `slug()` and `navigation_label()` have working defaults. Override only to rename.
- `navigation()` curates this resource's sidebar entry: override it to change the label, the `order`
  (lower renders first, ties keep declaration order) or the URL, e.g.
  `NavigationItem { order: -1, ..NavigationItem::for_resource::<Self>() }`. The URL is the Panel's
  call: `for_resource` names none, so the panel that mounts the resource resolves it to
  `{prefix}/{slug}`, and a resource never links at `/admin` on a panel mounted elsewhere. Spell a
  URL out instead (`NavigationItem::at(..)`) only to link somewhere other than the resource's list
  page — the Panel keeps it verbatim.
- `query()` is the seam for the resource's **own** row scoping — soft deletes, row-level visibility,
  and the relations a page loads. It is the full base query: the detail page and app code that calls
  `scoped_query` use it. Tenancy is not its job: when `requires_tenant()` is `true` the framework
  derives the `tenant_id` filter from the model's own schema and ANDs it onto whatever `query`
  returns, at every loader (GH #223), so restating it here is redundant.
- `query_with(cx, needs)` is the same base query narrowed to the relations a loader declared
  (GH #298). The default ignores `needs` and returns `query(cx)` unchanged, so a resource that
  overrides nothing loads the full base query at every loader. Override it to split the base query
  into one branch per declared name: a loader then loads an include only when it asks for it by name.
  The list and the export pass their table's `Table::include_needs()` (the union of the columns'
  `TextColumn::needs(..)` declarations); the edit page, delete, the bulk fetch, the unique probe,
  the relationship option lists and their targeted FK existence check, and the pagination probes
  pass an empty set. Keep the resource's own scope and whatever your `can_view` reads in every
  branch — the *tenant* half is not the override's to keep, the framework ANDs it onto what
  `query_with` returns exactly as it does for `query`. The option loaders run `query_with` with an
  empty set, so an option label must project the related record's own columns.
- `export_query(cx, needs)` is the export's seed; its default delegates to `query_with`, so
  overriding `query_with` narrows the export too (GH #177, GH #298, ADR-0018). Override
  `export_query` itself only when the CSV needs a branch the other loaders do not.
- `table()` and `form()` are hand-written, and so is the impl itself: a resource is `type Model` plus
  whichever hooks it uses. There is no `Resource` derive (GH #222) — the macros crate ships
  `derive(EmbeddedForm)` only (GH #191).

```rust
struct UserResource;

impl Resource for UserResource {
    type Model = User;
}

struct PostResource;

impl Resource for PostResource {
    type Model = Post;

    // the resource's own scoping seam, spelled out where it is used
    fn query(cx: &Cx) -> Query<List<Post>> {
        // soft deletes, row-level visibility, includes — not the tenant filter
    }
}
```

- Record fns (`create_record`, `update_record`, `delete_record`, `bulk_delete_records`) do the
  writes. Handlers load records, check policy, then call them in a transaction. `create_record` and
  `update_record` return the row they wrote — `toasty::create!` hands the created one back and a
  Toasty instance update reloads the model, so both are already in hand — because that is the only
  way the framework can name what a write committed (GH #112). A record fn therefore ends with
  `Ok(rec)` (at the end of an instance update that is usually the whole change), and a model used by
  a `Resource` derives `Clone`.
- `after_commit(cx, committed)` is the post-commit seam (GH #112): called once per committed write,
  after the transaction and before the response, with a `Committed` naming the mutation
  (`Mutation::Create/Update/Delete`) and the rows it wrote (a bulk delete is one call with all of
  them). It is where email, webhooks, audit rows and cache invalidation belong — running them in a
  record fn leaks the effect on a rollback, and the transaction's pool discipline forbids a second
  handle while it is open. The default is a no-op, a hook failure is logged without touching the
  committed write, and it never runs when nothing committed.
- For edit forms, `hydrate_form_values(cx, record)` maps a record to initial field values. `cx` is
  the request's: a scalar projection needs nothing from it, but an embedded value's keys come from
  the compiled mapping (GH #191).

## Tenancy

Tenancy is declared, not restated per query. The pattern:

```rust
impl Resource for PostResource {
    type Model = Post;

    // `Post` declares `tenant_id: uuid::Uuid`. Declaring this is the whole
    // tenant contract: the gate is GH #87 — every handler 403s without a
    // tenant — and the scope is GH #223 — every loader (list, edit, delete,
    // bulk, export, relationship options) ANDs `tenant_id = <request tenant>`,
    // derived from the model's own schema, onto whatever `query` returns.
    fn requires_tenant() -> bool {
        true
    }

    // The resource's *own* scoping only — a soft delete, row-level visibility,
    // the includes a page loads. Writing the tenant filter here is redundant:
    // the framework derives it at every loader, so a copy that disagreed with
    // the derived column could only hide rows, never widen access.
    fn query(_cx: &Cx) -> Query<List<Post>> {
        Query::<List<Post>>::all()
    }
}
```

App code that loads rows outside the framework's loaders — a record fn double-checking a foreign
key, a custom page — calls `scoped_query::<PostResource>(cx)?` rather than `PostResource::query(cx)`:
on a gated resource that method is the **tenant-unscoped** base, deliberately, so the
tenant-unscoped case is visible at the call site (it still carries whatever the resource's own
`query` scopes — soft deletes included).

Three shapes, one gate:

1. **The model carries the tenant** (the common case): `requires_tenant() =
   true` and nothing else. The framework derives `tenant_id = <request tenant>`
   from the model's own schema.
2. **The row inherits its tenant** — `Comment` has no `tenant_id`, it belongs to
   a post that does. Declare the predicate instead of the column:

   ```rust
   fn requires_tenant() -> bool {
       true
   }

   fn tenant_scope(tenant: uuid::Uuid) -> Option<toasty::stmt::Expr<bool>> {
       // The framework ANDs this onto `query`/`export_query` at every loader,
       // exactly as it ANDs the derived filter elsewhere.
       Some(Comment::fields().post().tenant_id().eq(tenant))
   }
   ```

   The gate is what matters: writing this filter inside `query` with
   `requires_tenant() = false` would *skip* it for a tenantless request rather
   than refuse it, serving every tenant's rows to a tenantless admin.
3. **The resource must serve more than one tenant** (a deliberate cross-tenant
   view): `requires_tenant() = false` and scope in `query` by hand — the one
   explicit way out, and the gate goes with it.

A gated resource that supplies no predicate at all — no `tenant_id` column and
no `tenant_scope` override — fails `Panel::build` (GH #231), the same boot
failure any other misdeclaration gets, and every loader keeps answering an error
naming itself rather than querying tenant-unscoped — the backstop for a
predicate that is only `None` for some tenants. There is no override that
removes the scope.

The request's tenant comes from the logged-in user; see
[Policy, auth, tenancy](./policy-auth-tenancy.md).
