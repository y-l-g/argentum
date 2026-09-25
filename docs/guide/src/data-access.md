# Data access

Querying Toasty from panel code: getting the `Db`, filters and sorting, preloading relations,
embedded values, and the render and reactivity invariants any page has to respect.

Get the DB from app context:

```rust
let mut db = tablo_core::db::db(cx);
let rows = User::all().exec(&mut db).await?;
```

`Db` is `Arc`-pooled; cloning per request is cheap and `exec` needs `&mut Db`.

Filter and sort:

```rust
User::filter(User::fields().email().eq("alice@example.com"))
User::filter(User::fields().name().starts_with(q))
    .order_by(User::fields().name().asc())
```

Table search builds its pattern through `like_with_escape` with `%`, `_` and the escape character
escaped (`escape_like_pattern`), so it is parameterised and portable. If you hand-write a pattern,
escape `%` and `_` first, and never interpolate raw input into SQL.

Preload relations in one trip:

```rust
let posts = Post::all()
    .include(Post::fields().author())
    .exec(&mut db)
    .await?;
// then `post.author.get()` with no extra query
```

Guard computed cells against a missing preload so a query change fails loudly, not with blank data —
and declare the relation, which is what the CSV export's narrowed query is built from (GH #177):

```rust
TextColumn::computed("Author", |p: &Post| {
    if p.author.is_unloaded() { "(unloaded)".into() } else { p.author.get().name.clone() }
})
.needs(["author"])
```

## Embedded values

Derive the codec and declare nothing per field (GH #191, ADR-0019). The derive reads the type's
shape, the framework names the columns:

```rust
#[derive(Debug, Clone, toasty::Embed, tablo_core::EmbeddedForm)]
pub enum Publication {
    #[column(variant = 1)]
    Scheduled { #[shared(timestamp)] scheduled_at: String, scheduled_for: String },
    #[column(variant = 2)]
    Published { #[shared(timestamp)] published_at: String, canonical_url: String },
}

// form declaration: controls, flattened names, and the variant Select
Section::new("Publication").schema(Publication::form(cx, Post::fields().publication()))

// hydration and the record fn: the typed value, keys resolved from the schema
write_embedded(cx, Post::fields().publication(), &record.publication, &mut values);
let publication = read_embedded(cx, Post::fields().publication(), &values);
if submitted(cx, Post::fields().publication(), &values) { /* the submit mentioned it */ }
```

An enum's variant is its **discriminant column**, carried by the form as a `Select` over the
schema's variant list — each option submitting the variant's stored value and reading as its name —
and each variant's payload renders inside its own marked group, so the client shows only the chosen
variant's, and a variant can be picked on create and changed on edit (a read-only page names the
stored variant instead of printing its discriminant). A named discriminant always wins (and one the
enum does not declare is refused loudly, never read as some other variant), so a stale payload is
not a vote. Only when no discriminant is named at all — the create form, a hand-written POST — do
payloads select one, by a variant's own **non-shared** payload through resolved keys. The toggle is
markup-only (`variant.js` hides the inactive groups): with JavaScript off every variant's payload
renders, so no field the server still parses is lost. `#[form(label = "…")]`, `#[form(textarea)]`
and `#[form(textarea, rows = N)]` are the per-field overrides; an unknown key is a compile error. A
`#[document]` inside a value, a relation, an enum nested inside an enum variant, and a tuple struct
are not covered.

Schema setup: `db.push_schema().await` for prototypes, `toasty-cli` migrations for prod.

## Render invariants

Pages, layouts, and components are side-effect free and deterministic — no `HashMap` iteration,
`Utc::now()`, or random IDs in a streamed region (breaks concurrent/streaming re-renders). Query
Toasty directly with explicit `include` for relations, `#[index]` for filter columns, and
`#[memoize]` for shared loads. `Cx`-scoped values (`Tenant`, auth), not middleware, carry request
scope. Every value read on the server via `get()` / `read()` is untrusted client input.

## Reactivity

`signal(cx, init)` runs only in a page/layout/component body; loop bodies need `#[key(...)]` and
reorderable rows need a stable `id` from the row key. `get()` / `read()` re-runs track and morph in
place; `get_untracked()` opts out. Page/layout guards do not run on shard requests — shards
authorize themselves (`requires_tenant` + `can_view_any` + the tenant-scoped query).
