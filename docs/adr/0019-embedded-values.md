# Embedded values: a derived codec, and the discriminant column as the variant rule

Date: 2026-09-22 — Status: accepted — Amended: 2026-09-22

## Decision

**1. The codec is derived from the type's shape; the keys come from the schema.**
`#[derive(EmbeddedForm)]` (in `argentum-macros`) generates the flat-map ↔ typed conversion, the
presence question, and a `form(cx, parent)` returning the value's controls. The app declares the value
per type — one derive, no field bindings — and calls `write_embedded` / `read_embedded` / `submitted`
where it hydrates and writes. It never spells a column: each leaf is addressed by a typed path
(`path_field`, variant-rooted for payloads) and resolved by the framework (`leaf_key`, `enum_spec`).
The derive supplies the Rust shape, the schema supplies the storage, and neither re-derives the other.

**2. A field is a leaf or a value, decided at macro time.** A type the panel can spell — `String`, plus
every type with a `TypedValue` impl (`i8`…`i128`, `isize`, `u8`…`u128`, `usize`, `f32`, `f64`, `bool`,
`Uuid`, `jiff::Timestamp`) — is one column; anything else (a relation, an `Option<T>`, a `#[document]`)
fails at that bound rather than binding quietly. Per-field overrides are `#[form(label = "…")]`,
`#[form(textarea)]` and `#[form(textarea, rows = N)]`; an unknown `#[form(..)]` key is a compile error.

**3. The variant is the discriminant column.** An enum's `write_form` writes the discriminant and the
active variant's leaves; its `read_form` reads the variant from the submitted discriminant in this
order:

1. A discriminant the submission names always wins, and one the enum does not declare is refused
   loudly (`read_form` panics) rather than read as some other variant, which would store a value the
   caller never asked for — a stale payload is not a vote.
2. Only when no discriminant is named at all — the create form, which has no stored variant to
   hydrate, or a hand-written POST — the first variant, in declaration order, with a payload **of its
   own** submitted. A `#[shared(..)]` column belongs to several variants, so it never selects one. The
   rule is reimplemented through the keys the schema resolves (`leaf_key`, and the nested value's own
   `any_present`) instead of remembered column names, so renaming a payload cannot change its meaning.
3. Otherwise the first variant.

**4. The variant control rides the form as a `Select` over the discriminant column.**
`discriminant_select` renders one option per variant the schema declares — each submitting the stored
value and reading as the variant's name (`value_of_index` / `name_of_index`) — and the derive wraps
each variant's payload in `Group::variant(discriminant, value)`, which renders `data-variant-select`
on the control and `data-variant` / `data-variant-of` on the groups. `assets/variant.js` (registered
in `xtask`'s `ASSET_FILES` / `ASSET_HOOKS`) hides the groups whose marker is not the control's value,
scoped to the form so two enums never toggle each other. It is markup-only, so with JavaScript off
every group renders and nothing the server parses is lost; a create form opens on the empty choice
(`-- Select --`), and the control is deliberately not `required`, so an empty submit still reaches
rule 2's payload fallback. A `#[shared(..)]` column renders once, outside every group, because it
belongs to several variants and must stay editable whichever one is chosen; only a variant's own
payload goes inside its group. A unit variant gets its group too, so the marker set is the schema's
variant list and a variant added later cannot silently lose its group
(`the_variant_groups_are_exactly_the_schemas_variants`).

**5. Hydration takes the request context.** `Resource::hydrate_form_values(cx, record)` — its keys come
from the compiled mapping, which lives on the request's app schema; the alternative, the app spelling
flattened names, is what the derived codec exists to remove. This is a breaking signature change for
every resource, mechanical (`_cx` where unused) and documented as the upgrade cost.

**6. What is not covered is part of the decision.** A `#[document]` inside an embedded value (its
fields share one column: the walk refuses rather than hand one column back for several fields), a
relation inside one, an `Option<T>`, an embedded enum nested inside an enum *variant* (value resolution
starts at a model root; nesting inside structs works at any depth), and a tuple or unit struct. A
derived form's labels default to the humanized Rust field name (`Seo Title` → `Title`) and are
overridable per field, and `Schema::extend` exists because `IntoSchema`'s tuple form stops at four
nodes.

## Consequences

- The showcase's embedded form sections are four declarations (`Seo::form(..)`, `Publication::form(..)`,
  `Media::form(..)`, `PostStats::form(..)`), and its update path stops spelling flattened column names
  to decide whether a value was submitted.
- **Editing keeps the stored variant** because the browser carries the discriminant back; a hand-written
  POST that names one switches it (`post_edit_switches_the_publication_variant_explicitly`).
  **Creating** works as it must: the create form has no stored variant, so rule 2 selects the variant
  its payload names (`post_create_keeps_the_variant_its_payload_names`).
- Derived controls are not required, because every column under an embedded step is storage-nullable —
  a declaration change, not a validation change, since the flags resolve identically. A `Textarea`
  keeps its height through `#[form(textarea, rows = 3)]`.
- The read-only page names the stored variant (`Published` / `Archived`) instead of printing its
  discriminant; that row says which state the record is in, and a record with no stored variant renders
  no row at all (ADR-0016).
- It is a visible breaking change: the discriminant is a visible `Select`, not a hidden input, so an app
  or test reading the form markup for it updates. The submitted value, `read_form`, the
  unknown-discriminant refusal and the fallback are unchanged.
