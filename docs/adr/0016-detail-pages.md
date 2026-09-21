# Detail pages: `Resource::view` and one Schema, read two ways

Date: 2026-09-21 — Status: accepted — Supersedes: none

## Context

`Panel::resource` has always registered `{slug}`, `{slug}/create` and
`{slug}/{id}/edit` — there was no page that just *showed* a record. The
criterion in the closed #66 ("`GET /admin/posts/{id}` shows `comments` without
per-row `exec`") was never implemented, and #113 tracked the gap. Two shapes
were available: add a parallel read-only vocabulary (Filament's infolist —
`TextEntry`, `ImageEntry`, …), or render the `Schema` a resource already
declares for its form in a read-only mode. The panel's whole design leans on
one vocabulary per concern (ADR-0007, ADR-0010): a second type per field is a
second thing to keep in step, and the field types' metadata (lens-derived
label, nullability, option labels) is exactly what a reading of a record
needs.

Two smaller forks came with it. The route `{slug}/{id}` shares its segment
position with the literal `{slug}/create`, and `{id}` prefixes `{id}/edit`.
And a row's `View` link must not appear for a resource that declares no view,
without a second declaration to keep in step with `view()`.

## Decision

**One Schema, rendered read-only.** `Resource::view(cx) -> Schema` defaults to
`Schema::empty()`; a resource that does not override it renders no page, links
no `View` action, and 404s the route. Rendering threads a mode rather than a
parallel field set: `RenderSource` gains `mode: Mode` (`Form` | `View`),
`Schema::render_readonly` is the entry point, and each field type branches in
`render_with` — `TextInput`/`Textarea` render a label and the stored value,
`Select` resolves a static option label to the label the form offered (and
renders the stored value when no option matches — a relationship key included),
`FileUpload` renders its path. Layout keeps the structure it declares: `Grid`
stays a grid, `Section`/`Group` keep their chrome.

Error slots are empty in view mode by construction: `static_errors` returns
nothing, so a stored record cannot render as invalid. A `Repeater` renders
through `render_source` rather than `render_with`, which is how it first
shipped a required `*` and `aria-invalid` onto a read-only page — it now has
its own view branch, and reads errors through `RenderSource::ignores_errors`
so a second layout cannot forget the rule. That near-miss is the argument for
the chokepoint, not against the mode: a field type that forgets its branch is
visible in the rendered page, and `readonly_render.rs` covers each one.

**`viewed(cx)` is derived, never declared.** `Resource::viewed` is
`!Self::view(cx).is_empty()`, so the row link, the handler's answer, and the
schema that renders the page cannot disagree. `Panel::resource` registers the
detail route unconditionally — it runs at build with no request, so `R::view(cx)`
is not declarable there — and the handler 404s a resource that declares no view.
That is the same answer an unknown id gets, costs one comparison, and keeps
"declares nothing" honest for a hand-typed URL.

**The route rides the router's own precedence.** topcoat routes through
`matchit`, whose documented behaviour is that a static segment outranks a
parameter one and that a longer path outranks a shorter prefix — independent of
registration order. `/admin/posts/create` therefore still reaches the create
page, and `/admin/posts/{id}/edit` still reaches the edit page. This is pinned
by `the_detail_route_does_not_shadow_create_or_edit` rather than left to the
dependency's docs.

**The page loads through the one query seam.** The detail GET uses
`find_by_key` (`R::query` + PK filter, ADR-0002) like the edit GET, so tenancy,
soft-delete scoping, and the 404 for an unknown *or* out-of-scope id come from
the seam rather than a second implementation. `can_view` on the loaded record is
a 403, not a 404: the record exists and this caller may not see it.

**Relations are preloaded but not yet rendered.** `query`'s `include` is what
makes the list's computed columns one round-trip; the detail page reuses it, so
the author and comments are already in hand. No declarative field reads a loaded
relation yet, so the showcase's post view renders the record's own scalar
columns, and a unit test pins that the `(unloaded)` marker the list columns use
never reaches the page. The field type that reads a relation from the loaded
record is the follow-up; the decision here is that it belongs in the *same*
Schema, not in a parallel page API.

## Consequences

- `GET {prefix}/{slug}/{id}` serves a detail page for any resource that declares
  a `view`; `Table::with_view` adds the row link, and only for those resources.
- A view can show any field the *form* can bind, which today means `String`
  lenses: typed columns (`Uuid`, `jiff::Timestamp`) need GH #192, and a relation
  needs the follow-up field type. The showcase documents both gaps where a
  reader hits them.
- Values come from `hydrate_form_values`, so a field that hydrates for the form
  is a field that renders on the page — but a resource that hydrates nothing
  (`UserResource`'s default is the empty map) renders a view of empty labels.
  That is a declaration error the page cannot detect; a resource declaring a
  view is expected to hydrate, as it must for its form.
- `Schema::is_empty` became public: `viewed()` reads it, and a caller deciding
  whether to link a page needs the same answer the framework uses.
- The tuple limit on `IntoSchema` (four) is now visible to consumers: a view
  with more than four top-level blocks needs a `Group` wrapper. The showcase's
  Post view hit it and says so.
