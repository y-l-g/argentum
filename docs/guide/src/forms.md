# Forms

Create and edit forms: typed lenses, layout blocks, the fields and what each one guarantees, file
uploads, and how validation failures are reported.

Forms use typed lenses, not string paths:

```rust
Schema::new((
    Section::new("Account").schema((
        TextInput::r#for(User::fields().email()).email().unique(),
        Select::r#for(User::fields().role())
            .options(vec!["admin".into(), "member".into()]),
    )),
    Grid::new(2).schema((
        TextInput::r#for(User::fields().name()),
        FileUpload::r#for(Post::fields().image_path()),
    )),
))
```

What to know:

- Layout blocks: `Section`, `Group`, `Grid`, `Tabs`. Fields: `TextInput`, `Textarea`, `Select`,
  `FileUpload`, `Repeater`. Every field takes a typed lens (`User::fields().email()`), never a string
  path. `Textarea` is the multi-line half of `TextInput` — same lens, same required default, same
  error contract, a `<textarea>` control instead, and no `unique()` (the app-side probe is built from
  `TextInput`, GH #184).
- `required` defaults to the column nullability. Use `.optional()` to opt out. A bare `Select` over a
  non-nullable FK rejects `""` inline instead of failing at the driver.
- `email()` applies the `email_address` grammar at the form edge: a text domain needs two labels
  (`a@b` and `a@b..c` are refused), a display name is a header rather than an address, and the whole
  address is capped at 254 octets (RFC 5321 §4.5.3.1.3). A quoted local part
  (`"a b"@example.com`), a unicode address (`用户@例え.jp`) and a bracketed domain literal
  (`a@[127.0.0.1]`) pass.
- **A non-`String` column binds through `TextInput::typed`** (GH #192):
  `TextInput::typed::<Post, i64>(Post::fields().post_stats().word_count())` renders the value's
  `Display`, parses the submission through the type's own `FromStr`, and refuses what it cannot parse
  as an inline field error naming the input — `` `twelve` is not a valid whole number `` — instead of
  a 500 or a silent default. What is stored is `Display` of the parsed value, so a value re-submitted
  unchanged is written back in the shape it was read. `typed_context(cx, path)` is the
  embedded/document sibling, as `r#for_context` is to `r#for`. `TypedValue` covers the integer types,
  `f64`, `Uuid` and `jiff::Timestamp`; a type needing its own words implements the trait. Empty is
  the presence rule's business, not the typed one: a typed column has no spelling for "no value", so
  an empty submission on an optional typed field reaches the record fn as `""` exactly as any other
  optional column does.
- `unique()` does two things. It adds an app-level pre-check — Toasty exposes no unique-violation
  predicate yet, so the DB constraint stays the final guard and concurrent writes can race — and it
  implies **presence**: the framework stores `""` rather than NULL, so an empty value on a unique
  field is refused inline as `"<Label> is required"` instead of being written past an index that
  admits only one (GH #189). `.optional()` does not lift that rule, and `Panel::build` refuses a
  `unique()` marker on a column with no unique index (single-field or composite, `#[unique(a, b)]`
  included), so the declaration and the database cannot disagree about which fields are unique.
- Relation select validates the FK against the related resource query before `create_record` runs:

```rust
Select::r#for(Post::fields().author_id())
    .relationship::<AuthorResource>(
        AuthorResource::query,
        |a: &Author| a.id,
        |a: &Author| a.name.clone(),
    )
    .label("Author")
```

- Relation options are bounded to 200 (`MAX_RELATIONSHIP_OPTIONS`) and memoized per
  `(request, tenant)`. Small tables validate against the bounded set; `can_view` filters before
  labels, `can_view_any`/tenant denial fails closed (`not available`).
- Large reference tables (10k+ rows) need `.searchable()` on the `Select` (GH #150): over-cap
  searchable selects degrade to type-to-search instead of a retry error. Typing fetches
  `GET {parent_list_url}/options?field=&q=` (debounced 200ms, abort in-flight, selection preserved),
  which reuses the related `Table`'s declared `searchable()` columns (`search_expr`), bounds to 200,
  and filters `can_view` before labels. No searchable columns → hard-cap path (non-searchable keeps
  the cap error). Overflowed submits validate via a targeted PK check (the tenant-scoped query +
  `can_view`): legitimate FKs beyond the cap pass, hidden → `invalid`, denied → `not available`, DB
  failure → retry. Initial render keeps the stored value + search input + “Too many options — type
  to search” hint; no-JS keeps the plain select (other fields still submit, relation cannot be
  changed past the cap).

- `FileUpload` binds a `String` path and owns the request half: no `value` on `type=file`,
  `enctype="multipart/form-data"` when a form has one, a 10 MiB body cap (413), a 400 for multipart
  without a boundary, and sanitized basenames (`.` / `..` / Windows reserved names surface as inline
  errors). On an edit the control drops native `required` (GH #184) — a `required` file input cannot
  be pre-filled, so it blocked every untouched save; `required` still holds on create.
- **Where the bytes go is the app's** (GH #188, ADR-0017): install an `Uploader` once with
  `Panel::uploads(store)`. `store(filename, bytes) -> Result<String, String>` receives the sanitized
  name and the content (bounded by the cap) and returns the value the record stores; a refusal is an
  inline error (`"<Label> could not be uploaded: <reason>"`), not a 500. With no uploader installed
  the sanitized basename is stored — the default — and the bytes are drained rather than buffered.
- The stored path renders as a link to the file, on the edit form and on the detail page (GH #242):
  the framework reads no extension and renders what the app stored, inventing no URL convention.
  `Panel::serve_dir(path, dir)` mounts the directory an upload store writes to. A served directory is
  **public** (ADR-0017): its URLs answer whoever asks, with no session, because the auth gate covers
  only the panel prefix and `/_topcoat/runtime` (ADR-0013) and a served directory is mounted outside
  both. An app that needs protected files owns that route itself. Every stored value also offers a
  `clear_<field>` checkbox ("Remove the current file"), the one control that means "remove" rather
  than "keep"; an empty file input still means keep. Clearing does not waive `required` — a resource
  whose records may lose their file declares `.optional()`.
- `Repeater` is a single-entry group. An all-empty group is skipped, so its inner required fields do
  not fail the submit. A `required` repeater yields one label-keyed error; a partially filled group
  still enforces inner `required`.

Validation errors render inline per field. Absent keys validate as `""` and updates write only
present keys; handlers reject unknown form keys with 400 (`role` / `tenant_id` smuggling fails
closed; only `csrf_token` and `clear_<field>` are exempt), so extra posted keys never reach record
fns.

Embedded values declare one form node and flatten their fields; see
[Data access](./data-access.md#embedded-values).
