# Detail pages: `Resource::view` and one Schema, read two ways

Date: 2026-09-21 — Status: accepted — Amended: none

## Decision

**One Schema, rendered read-only.** `Resource::view(cx) -> Schema` defaults to `Schema::empty()`; a
resource that declares a view links a `View` row action and serves the page. Rendering threads a mode
rather than a parallel field set: `Schema::render_readonly` is the entry point and `Mode::View` rides
`RenderSource`; each field type branches — `TextInput`/`Textarea` render a label and the stored value,
`Select` resolves a static option label (falling back to the stored value, a relationship key
included), `FileUpload` renders its path — while layout keeps the structure it declares (`Grid` stays
a grid, `Section`/`Group` keep their chrome). Error slots are empty in view mode by construction:
`RenderSource::errors_for` returns nothing in `Mode::View`, the one place that rule lives, so a layout
cannot forget it and a stored record cannot render as invalid.

**`viewed(cx)` is derived, never declared.** `Resource::viewed` is `!Self::view(cx).is_empty()`, so the
row link, the handler's answer, and the schema that renders the page cannot disagree.
`Panel::resource` registers the detail route unconditionally — it runs at build with no request, so
`R::view(cx)` is not declarable there — and the handler 404s a resource that declares no view, which
is the same answer an unknown id gets.

**The route rides the router's own precedence.** topcoat routes through `matchit`, whose documented
behavior is that a static segment outranks a parameter one and a longer path outranks a shorter prefix
— independent of registration order — so `/admin/posts/create` still reaches the create page and
`/admin/posts/{id}/edit` still reaches the edit page. This is pinned by
`the_detail_route_does_not_shadow_create_or_edit`.

**The page loads through the one query seam.** The detail GET uses `find_by_key` (the tenant-scoped
query plus a PK filter, ADR-0002) like the edit GET, so tenancy, soft-delete scoping, and the 404 for
an unknown *or* out-of-scope id come from the seam rather than a second implementation. `can_view` on
the loaded record is a 403, not a 404: the record exists and this caller may not see it.

**Relations render from the record, beside the Schema.** The query's `include` already loads the
related rows for the list's computed columns, and the detail page reuses it. They cannot go *in* the
Schema: `Resource::view(cx)` takes no record — it is a declaration, read at build time as well as per
request — and a `Schema` renders the record's string projection (one `HashMap<String, String>`) while
a relation is a list of records, so a relation node would need the render tree to carry a record of
the caller's type. Instead the detail page has a second, typed half:
`Resource::view_relations(cx, record) -> Option<BoxView>`, rendered under the Schema's fields, so the
record arrives with its type intact and a relation renderer can take typed column projections with no
erasure. That hook is also where the no-N+1 contract lives: it reads the rows the one `include`
loaded and issues no query of its own; `is_unloaded` is the guard the framework's own columns use, and
the showcase counts the statements a detail page runs with and without related rows.

## Consequences

- `GET {prefix}/{slug}/{id}` serves a detail page for any resource that declares a `view`;
  `Table::with_view` adds the row link, and only for those resources.
- A detail page is the Schema *plus* `view_relations`, so a resource with related rows to show
  declares both; `view` alone is a page of scalar fields.
- A view binds the same fields a form can — `TextInput::r#for` for `String` lenses, `TextInput::typed`
  (GH #192) for the rest — while a relation renders through `view_relations` rather than as a Schema
  field. The showcase documents both where a reader hits them.
- Values come from `hydrate_form_values`, so a field that hydrates for the form renders on the page; a
  resource that hydrates nothing renders a view of empty labels, a declaration error the page cannot
  detect.
- `Schema::is_empty` is public: `viewed()` reads it, and a caller deciding whether to link a page needs
  the same answer the framework uses.
- The tuple limit on `IntoSchema` (four) is visible to consumers: a view with more than four top-level
  blocks needs a `Group` wrapper.
