# Architecture

How the crates fit together, what happens on a request, and where the extension points are.
`CONTEXT.md` defines the vocabulary; `docs/guide/` explains how to use the toolkit.

## Crates

`argentum-macros` and `argentum-ui` are leaves. `argentum-core` depends on both, and the showcase
depends on `argentum-core` and `argentum-ui`.

| Crate | Depends on | Contents |
| --- | --- | --- |
| `argentum-macros` | — | the `EmbeddedForm` derive |
| `argentum-ui` | `topcoat` | synced primitives, owned composites, `icons.rs` |
| `argentum-core` | `argentum-macros`, `argentum-ui`, `toasty` | Panel, Resource, Table, Schema, auth, tenancy, upload |
| `examples/showcase` | `argentum-core`, `argentum-ui`, `toasty` | the runnable admin and the integration tests |

`argentum-core` never depends on a concrete database driver. Everything reaches the database through
Toasty's `Db` and `Executor`, which is why an app-level `Uploader` and the `Authenticator` are traits
the app implements rather than crates the toolkit picks.

## The layering

```
Panel  ──declares──▶  Resource  ──declares──▶  Table   (the list view)
   │                      │                  └▶  Schema  (forms, detail pages)
   │                      └──record fns─────▶  create / update / delete
   └──owns──▶ Router, Db in app context, Shell, the auth gate
```

A `Panel` owns the router, the `Db` in app context, the shell layout, and the authentication gate.
Registering a `Resource` on it adds that resource's routes and its sidebar entry. A `Resource` maps
one Toasty model to its admin UI: a base query, a `Table`, a `Schema`, a policy, and the record
functions that perform writes.

`Table` and `Schema` are declarations, not renderers. The Panel calls `table()` and `form()` once at
boot, so they must not need request-scoped context; a declaration that cannot render fails
`Panel::build` rather than a request.

## A read request

A resource list page runs, in order:

1. `enforce_auth(cx)` — resolve the session, or redirect to the login page.
2. `enforce_tenant::<R>(cx)` — refuse with 403 when `R::requires_tenant()` and the request has no
   tenant.
3. `R::can_view_any(cx)` — the list-level policy check, before any row is loaded.
4. Parse `TableState` from the URL (`?q=`, `?sort=`, `?dir=`, `?after=`, `?filters=`, `?group_by=`).
5. Load through `scoped_query::<R>(cx)`, which is `R::query(cx)` with the framework's tenant filter
   ANDed on.
6. Render the table inside a `suspense` region: the skeleton is sent with the shell, the loaded rows
   swap in.

The list checks `can_view_any` only, so pagination stays honest; per-row `can_view` trims the export
and the relationship option lists. A detail page loads through the same scoped query, so an unknown
id and one outside the tenant are the same 404, while a row the caller may not view is a 403.

## A write request

Create, update, delete, and bulk delete run the same shape:

1. `enforce_auth`, `enforce_tenant`, and the matching `can_*` check.
2. For a form: validate, which also resolves relationship fields against the related resource's
   query.
3. Open a framework-owned transaction and re-load the target through the scoped query, so policy is
   checked against the row that is about to be written rather than the submitted id.
4. Call the resource's record function inside that transaction.
5. Commit, then call `Resource::after_commit(cx, committed)`.

Every POST carries a double-submit CSRF token, and a bulk delete additionally requires the
`confirm=1` marker that only the confirm control emits. The record functions are the mutation
vocabulary; a non-CRUD operation is a record function or a hand-written page.

`after_commit` is the only place for a side effect that must not survive a rollback — email, a
webhook, an audit row. It runs after the transaction and before the response, it runs once per
committed write, and a failure in it is logged without rolling the write back.

## Extension points

| Seam | Where | What it decides |
| --- | --- | --- |
| `Resource::query` | `resource/mod.rs` | the resource's own row scoping: soft deletes, row-level visibility, includes |
| `Resource::tenant_scope` | `tenancy.rs` | the tenant predicate, derived from the model's `tenant_id` by default |
| `Resource::export_query` | `resource/mod.rs` | the export's base query, narrowed to the includes its columns declared |
| `Resource::can_*` | `resource/mod.rs` | authorization, default deny |
| `Resource::editable` / `deletable` | `resource/mod.rs` | whether the row chrome renders, default off |
| `schema::OptionSource` | `schema/relationship.rs` | what a relationship select offers, and who may see it |
| `EmbeddedForm` | `argentum-macros` | the flat form map ↔ a typed embedded value |
| `Uploader` | `upload.rs` | where a `FileUpload`'s bytes go |
| `Authenticator` | `auth.rs` | how credentials resolve to a `CurrentUser` |
| `Table::id` / `Table::pk` | `resource/table/mod.rs` | row identity for keyed diffs and for action URLs |

Row identity is two projections and both are declared: `Table::id` is the display key that drives
keyed diffs and DOM ids, `Table::pk` is the record key that handlers resolve as the model's typed
primary key. Action chrome without `pk` is a render error, not a silent 404.

## Reactivity

The toolkit ships no client framework. Two Topcoat mechanisms cover the interactive parts:

- **`suspense`** streams a region's content after the first render. The resource list uses it so the
  page shell and skeleton arrive first and the table swaps in.
- **Shards** re-render a region in place. A table with `Table::live_search(true)` hands its chrome to
  the page's `TableSignals`: search, sort, filters, and pagination write signals, the shard re-renders
  the table, and Topcoat morphs the result in place so focus and scroll survive.

Page and layout guards do not run on a shard request, so a shard authorizes itself. Renders are
side-effect free and deterministic: no `HashMap` iteration, no `Utc::now()`, no random ids in a
streamed region.

## Assets

`argentum-ui` owns ten browser scripts under `crates/argentum-ui/assets/`. They are loaded through
`asset!`, so they have no build step. Each one is wired to a constant in `argentum-ui/src/lib.rs`,
and a test guards the pairing: `cargo test -p xtask` runs `shell_assets_match_hook_contract`, which
fails when an asset is missing or a hook no longer appears in both its JavaScript and the Rust that
renders it. `asset!` does not read its source at compile time, so nothing else checks the JavaScript
side of that coupling.

`cargo xtask sync-topcoat-ui` re-vendors `components/primitives/` from `topcoat-ui-registry` and
writes a content hash into each file header. Those files are never hand-edited;
`cargo xtask verify-topcoat-ui` fails on drift. Components in `components/composites/` are
Argentum's own and are never overwritten.

## Module map

```
crates/argentum-core/src/
  panel/      mod, list, forms, actions, detail, search, shell, headers
  resource/   mod, table/{mod,render,export}, column, state, filter, relation,
              navigation, naming, commit
  schema/     mod, fields, layouts, lenses, tree, relationship, embedded, pk,
              validation
  auth, csrf, cursor, db, notification, query_term, tenancy, upload
```

The three largest modules split along the request shape rather than by type: `panel/` holds the
handlers, `resource/` holds what a resource declares and how a list renders it, and `schema/` holds
the form and detail-page declaration and its rendering.
