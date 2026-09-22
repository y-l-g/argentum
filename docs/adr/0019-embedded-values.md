# Embedded values: a derived codec, and the discriminant column as the variant rule

Date: 2026-09-22 — Status: accepted — Supersedes: none — Amends: none (extends ADR-0011's `include`/`(unloaded)` seam with the value half)

## Context

Embedded **leaves** bind one column at a time (GH #185): a lens through an
embedded struct, an enum variant, or a `#[document]` resolves to its flattened
storage column, and a `TextInput` posts that column. The **value** those leaves
belong to had no seam at all. Every app reassembled it from the flat map by
hand, and the showcase's `embedded_from_values` showed what that cost:

- the same function had to be written per app, per type;
- an enum's variant was recovered from **which payload columns happened to be
  non-empty** — `if !canonical_url.is_empty() { Published } else if
  !reason.is_empty() { Archived } else { Scheduled }` — so renaming a payload
  silently changed the meaning, a stale payload outvoted the variant the user
  meant, and only one variant's fields could realistically be shown at a time;
- hydration was a second hand-written projection
  (`hydrate_form_values`), spelling the same flattened names from the other
  direction.

The framework could not infer any of it. Toasty exposes no instance→field
reflection (upstream #119), so the framework cannot read a model's fields
generically; and the *names* belong to the compiled mapping, which GH #185
already established as the only authority (name accumulation was removed on
purpose).

Two facts from the compiled schema make a real seam possible, and neither was
being used:

- `app::EmbeddedStruct` / `app::EmbeddedEnum` describe the whole value — its
  fields, its variants, each variant's payload range;
- `mapping::FieldEnum` carries a **discriminant column**
  (`publication` for `Post.publication`), and `app::EnumVariant` carries the
  value each variant stores there. The variant is *stored*, not implied.

## Decision

**1. The codec is derived from the type's shape, the keys come from the
schema.** `#[derive(EmbeddedForm)]` (in `argentum-macros`) generates the
flat-map ↔ typed conversion and a `form(cx, parent)` returning the value's
controls. It never spells a column: each leaf is addressed by a typed path
(`<T as Embed>::path_field::<FieldTy>(index)`, variant-rooted for payloads) and
resolved by the framework (`leaf_key`, `enum_spec`). The derive supplies the
Rust shape, the schema supplies the storage, and neither re-derives the other.

**2. A field is a leaf or a value, decided at macro time.** A primitive type
(`String`, the integer/float types, `bool`, `char`, `Uuid`, `jiff::Timestamp`)
is one column; anything else is another embedded value, delegated to that type's
own impl, so nesting composes. `#[form(leaf)]` / `#[form(embedded)]` override
the guess, `#[form(label = "…")]` and `#[form(textarea)]` cover the two UI
choices a type cannot make.

**3. The variant is the discriminant column, never payload emptiness.** An
enum's `write_form` writes the discriminant and the active variant's leaves; its
`read_form` reads the variant from the submitted discriminant. A submission
without one — a hand-written POST, or the create form before a variant control
exists — reads as the **first variant**, and payloads are never consulted. This
is the correction the issue asked for: a filled `canonical_url` is not a vote.

**4. The discriminant rides the form as a hidden control.** `TextInput::hidden`
renders a bare `<input type="hidden">` (nothing in view mode) so the stored
variant hydrates into the edit form, the browser posts it back, and the update
writes the variant the record already had — through the ordinary value map, with
no new field kind, no validation, and no read-only row.

**5. Hydration takes the request context.** `Resource::hydrate_form_values`
becomes `hydrate_form_values(cx, record)`. Its keys come from the compiled
mapping, which lives on the request's app schema; the alternative — the app
spelling flattened names — is exactly what GH #185 removed. This is a breaking
signature change for every resource, mechanical (`_cx` where unused) and
documented as the upgrade cost.

**6. The list of what is *not* covered is part of the decision.** A
`#[document]` inside an embedded value (its fields share one column, so there is
no typed projection to bind), a relation inside one, a tuple or unit struct, and
a variant **control** — every variant's payload still renders, and choosing one
in the UI needs a form-reactivity seam the panel does not have (the live binding
is `TextInput`-only, GH #154 §4). The last is the outstanding half of GH #191.

## Consequences

- `embedded_from_values` is deleted from the showcase, and
  `post_edit_binds_and_saves_embedded_fields` — the GH #185 round-trip test —
  passes **unchanged**: the flattened columns still reach the record fn, they
  are simply named by the framework now.
- The showcase's embedded form sections collapse from ~80 hand-written bindings
  to four declarations (`Seo::form(..)`, `Publication::form(..)`,
  `Media::form(..)`, `PostStats::form(..)`), and its update path stops spelling
  `seo_title` / `publication_timestamp` / `media_` / `post_stats` to decide
  whether a value was submitted (`submitted(cx, parent, values)`).
- **A behaviour change in the showcase until the variant control lands**: a
  browser edit can no longer switch `Publication`'s variant by filling a
  different variant's payload — the stored variant is what the form carries
  back. A hand-written POST that names a discriminant switches it (pinned by
  `post_edit_switches_the_publication_variant_explicitly`), and the follow-up
  `Select` is what gives a browser user the same power.
- `TextInput::hidden` is public and general, but has one caller: the
  discriminant. It exists as a `TextInput` rather than a new node so it rides
  `field_names()`, hydration, validation and re-render without widening the
  render tree.
- `Schema::extend` is added because `IntoSchema`'s tuple form stops at four
  nodes and a derived form has one control per leaf.
- A derived form's labels default to the humanized Rust field name, which is a
  small improvement over the flattened column name (`Seo Title` → `Title`) and
  is overridable per field.
