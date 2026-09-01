# Review feedback — tokio-rs/topcoat PR #373

**PR:** [`feat!: streaming SSR and live! + emit! regions`](https://github.com/tokio-rs/topcoat/pull/373) (`view-stream`, reviewed at head `30d92c99`, 2026-09-01)

Each entry below is a self-contained review comment: it should make sense on its own, pasted into the PR, without knowledge of the reviewer's project. Entries are numbered and added as they surface while porting a real consumer (a lazy-`View`-era admin framework on `topcoat` + `topcoat-ui-registry` + `topcoat-router`) to the branch; the entry list grows until the port compiles and passes its test suite. Nothing here is hypothetical — every entry was hit in practice.

Sections: [API design](#api-design) · [Diagnostics](#diagnostics) · [Docs](#docs) · [Bugs / behavior](#bugs--behavior) · [Praise](#praise)

---

## API design

### FB-01 — `Result` lost its default type parameter; every bare `-> Result` breaks with E0107

`main` declares the facade alias with a default:

```rust
pub type Result<T = view::View, E = topcoat_core::error::Error> = ...;
```

The branch drops the default:

```rust
pub type Result<T, E = topcoat_core::error::Error> = ...;
```

Consequence for any downstream code base of realistic size: every component and handler that returned bare `Result` (previously meaning `Result<View>`) now fails with `E0107: missing generics for type alias topcoat::Result`. In the consumer ported for this review that was **109 of the first 198 compile errors** — the single largest migration cost of the PR, and it is purely mechanical: none of those sites cares what the success type is, they just bubble views upward.

If the intent is "handlers should now return `Result<impl View>`", the default still has a role: `Result<View>` remains the natural return type for components whose success type is not worth naming (dynamic composition, trait-object-style returns, code not yet moved to `impl View`). Keeping `T = View` — or at minimum providing a named alias for it — would let this breaking PR land for consumers as two independent migrations (lazy `View`, then optional re-typing to `impl View`) instead of forcing both at once.

Suggestion: restore `pub type Result<T = View, E = ...>` (the `View` trait is nameable again now that it exists as a trait; a boxed-erased concrete alias is also fine), or add e.g. `pub type ViewResult = Result<Box<dyn View>>` and mention it in the migration notes.

### FB-02 — There is no idiomatic "list of views" anymore

On `main`, `Vec<View>` was a natural accumulator: collect rendered fragments in a loop, then interpolate them. With `View` a trait, the only stock types are:

- `BoxView<'a>` (`Pin<Box<dyn View>>`) — public in the crate root, but not documented as the way to hold "some views";
- `Child<'a>` — a single child, `Default` to empty, `From<View>`; there is no `FromIterator`, no concatenation, no way to interpolate a collection of children.

So `let mut items: Vec<View>` becomes either `Vec<BoxView>` (and then… how is that interpolated? — there is no documented `(items)` behavior for `Vec<BoxView>`) or a restructure of the whole component into a single `view!` with a `for` loop inside. The restructure is often the better code, but the PR leaves no breadcrumb that this is the intended shape, and `BoxView`/`ViewExt::boxed` docs describe boxing purely as a fix for "multiple `return` sites", not as the list accumulator.

Suggestion (any one of):

1. Document the intended pattern for "render a dynamic list" in `view.md` (a `for` loop inside a single `view!`, `key:`-ed), with an explicit note that `Vec<View>` accumulators from the eager era should be restructured.
2. Add `impl FromIterator<BoxView<'a>> for Child<'a>` (or a `fragment!` / `Child::list(...)` helper) so mechanically-ported code has a correct target.
3. If `Vec<BoxView>` interpolation already works in `view!`, document it — reviewers had to read the macro source to find out it does not (or does?).

### FB-03 — `Slot<'_>` is just an alias; the docs teach it as a separate concept

`topcoat-router` declares `pub type Slot<'a> = Child<'a>` (crates/topcoat-router/src/page.rs:135). Layout docs, error docs and the PR description all present `slot: Slot<'_>` as a new third kind of thing next to props and children, with its own interpolation rules. It is the same type as child content: same interpolation, same laziness, same `Default`.

This is a minor point — the alias is good for readability — but the docs never say "a `Slot` **is** child content; everything documented about `Child` applies", so a consumer who reads the component docs then the layout docs meets two vocabularies for one mechanism and assumes different semantics (e.g. whether a layout may forward its slot into a nested component — which works, precisely because they are the same type).

Suggestion: one sentence in `page.rs`'s `Slot` docs and in the layout section of the router docs: "`Slot` is `Child` under another name; see the child-content section of the `view!` guide."

---

## Diagnostics

### FB-04 — E0782 on `child: View` params offers no path to the correct fix

Pre-PR components declared child content as a plain parameter:

```rust
#[component]
async fn card(#[default] child: View) -> Result { ... }
```

Post-PR this is `E0782: expected a type, found a trait`, and rustc's structured suggestions are all wrong for a component parameter:

- `use a new generic type parameter, constrained by View` → `child<T: View>: T` (not valid at a component call site, and not how children work),
- `impl View` → changes the signature to one the `#[component]` macro does not collect children into,
- `&dyn View` → also not the mechanism.

The actual fix is `#[default] child: Child<'_>`, which is documented in the `#[component]` docs and examples but **never appears in the diagnostic**. In the consumer port, this was 89 sites; the fix was discoverable only from the PR description and the migrated registry sources.

Since `#[component]` rewrites the function anyway, the macro is well placed to intercept this exact pattern (a parameter named `child` whose type is the trait `View`) and emit a targeted error: "`child: View` is pre-lazy-View syntax; declare `child: Child<'_>` (usually with `#[default]`)". That single diagnostic would collapse the largest non-mechanical error class of this migration.

### FB-05 — Migration is discoverable only from the PR description

The PR prose is excellent, but once merged it lives in a commit message. Nothing in the repo teaches the old→new mappings a consumer needs:

- `child: View` → `#[default] child: Child<'_>`
- `slot: Result` (layout) → `slot: Slot<'_>` + `(slot)` interpolation
- matching on `slot: Result` in a layout to set status codes → `error_boundary` fallback
- `#[component(boxed)]` → `ViewExt::boxed()`
- `boundary`/`defer` skeleton regions → `suspense` / `live!` + `emit!`
- bare `-> Result` → `-> Result<impl View>` / `-> Result<View>` (see FB-01)
- raw handlers returning `ViewFuture<'_>` → `AsyncIntoResponse` (see FB-06)

Suggestion: a "Migrating from the eager view layer" section in `crates/topcoat/docs/view.md` with exactly this table, and a line in each removed item's former doc location pointing at its replacement (`suspense` docs saying "replaces the `defer`/`boundary` pattern" etc.). The new `examples/live`, `examples/suspense`, `examples/error` are great — they are how the reviewer learned the new signatures — but examples teach the target state, not the path.

---

## Docs

### FB-06 — `ViewFuture` is gone with no pointer to its replacement

`main` exports a `ViewFuture<'_>` (a boxed future yielding `View`), which was the documented return type for raw route handlers that resolve a view by hand. On the branch the type no longer exists. The replacement mechanism — handlers return any `AsyncIntoResponse`, and `ViewExt::first()`/`single()` resolve a view's content to something `IntoResponse` — is only inferable from `response.rs` source and the PR prose.

Concretely: a consumer with `fn handler(cx: &Cx, body: Body) -> ViewFuture<'_>` gets a bare "cannot find type ViewFuture" and no hint. The `AsyncIntoResponse` trait docs are good ("every `IntoResponse` type implements it") but never say "this replaces the old `ViewFuture`-returning handler style; resolve your view with `.first()`".

Suggestion: cover raw/low-level handlers in one place — either a short section in the router docs ("low-level handlers: return `impl AsyncIntoResponse`, resolve views with `ViewExt::first`/`single`") or a doc-alias on `AsyncIntoResponse` for `ViewFuture`.

---

## Bugs / behavior

*(none yet — entries added as found during the port)*

---

## Praise

Worth saying explicitly, because it shapes how the complaints above should be weighed:

- **The lazy-`View` + streaming model is the right call.** "Ship the shell, stream the slow part" is exactly the shape consumer UIs keep hand-rolling; building it into the response layer (no client library, marker comments + template splices) is the correct place for it.
- **`emit!` returning `Result<EmitToken>`** — making "a live region must emit at least once" a type-level constraint, and letting the body retry/fallback instead of killing the stream, is a genuinely elegant design.
- **`error_boundary` with rethrowable fallbacks** cleanly replaces the old match-on-`slot: Result` status-code dance and composes up the tree.
- **`#[component(boxed)]` → `ViewExt::boxed()`** moves the type-cycle hack from macro magic into visible, grep-able code.
- The rustdoc on the new pieces (`View`, `Child`, `ViewExt::first/single`, `suspense`, `error_boundary`, `live!`/`emit!`) is unusually good — the two-phase `poll_first`/`poll_swap` contract is explained clearly enough to reason about performance without reading the runtime.
- The migrated `topcoat-ui-registry` is internally consistent — a consumer syncing registry sources verbatim gets a working component library on the new API with zero manual patching.

---

*Entries FB-01…FB-06 were known before the port began (compile-error triage); later entries are appended in discovery order.*
