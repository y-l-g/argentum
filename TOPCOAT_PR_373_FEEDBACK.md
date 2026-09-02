# Review feedback — tokio-rs/topcoat PR #373

**PR:** [`feat!: streaming SSR and live! + emit! regions`](https://github.com/tokio-rs/topcoat/pull/373) (`view-stream`, head `30d92c99`).
Found while porting a real consumer (admin framework on `view` + `router` + `ui-registry`). Every entry was hit in practice. Ordered by value.

| # | Type | Summary |
|---|------|---------|
| 1 | API | Restore `Result`'s `T = View` default |
| 2 | API | Blanket `NodeClassify for T: View` — opaque `impl View` can't interpolate |
| 3 | API | Raw `PageFn` needs `internal::ThenView` for fallible pages |
| 4 | DX | E0782 on `child: View` never suggests `Child<'_>` |
| 5 | API | No idiomatic list-of-views |
| 6 | Docs | Publish an old→new migration table |
| 7 | Docs | State the borrowing rule: views may borrow `cx` only |

**Bugs found: 0.** Streaming, `suspense`, `live!`/`emit!`, error→status mapping and mid-stream redirects all behaved as documented.

---

### 1 · API — Restore `Result`'s default type parameter

`main`: `pub type Result<T = view::View, E>`. Branch drops the default → every bare `-> Result` fails with E0107. One consumer port: **109 of the first 198 errors**, all mechanical.

**Fix:** restore `T = View`, or add `pub type ViewResult = Result<View>`. Lets consumers adopt lazy views and `impl View` re-typing as two separate migrations.

### 2 · API — Blanket `NodeClassify for T: View`

`(expr)` interpolation only works for 5 hard-coded types (`Child`, `BoxView`, `ScopeView`, `MoveView`, `LiveView`). A helper returning `Result<impl View>` produces an opaque that implements `View` but not `NodeClassify` → every intermediate helper must `.boxed()` just to stay interpolable, and the error names the wrong trait (`NodeViewParts not implemented for impl View`).

**Fix:** `impl<T: View> NodeClassify for T`, or at least a diagnostic that says "box it with `.boxed()`".

### 3 · API — Raw `PageFn` can't express a fallible async page with public types

`PageRenderFn` is now sync: `fn(&Cx, Body) -> BoxView<'a>`. `#[page]` adapts async handlers via `topcoat_view::internal::ThenView` — consumers hand-registering pages (generic registries) must import an `internal` module and `Box::pin` by hand.

**Fix:** public constructor (`page_from_fn(.., |cx, body| async { Result<impl View> })`) or `pub use ThenView`.

### 4 · DX — E0782 on `child: View` offers no path to the fix

Pre-PR `#[default] child: View` was the standard child-content signature. Post-PR it's E0782 whose suggestions (`impl View`, generics, `&dyn View`) are all wrong; the right fix (`#[default] child: Child<'_>`) appears nowhere in the output. Largest non-mechanical error class of the migration (89 sites).

**Fix:** `#[component]` should intercept a `child`-named parameter typed as the `View` trait and emit "declare `child: Child<'_>` (usually with `#[default]`)".

### 5 · API — No idiomatic list-of-views

`Vec<View>` is gone. `BoxView` is the only escape hatch and is documented only as a fix for multiple `return` sites; `Child` has no `FromIterator`; nothing documents the intended pattern.

**Fix (any one):** `impl FromIterator<BoxView<'a>> for Child<'a>`, a `fragment!` helper, or a `view.md` note that eager-era accumulators should become `for` loops in one `view!`.

### 6 · Docs — Publish an old→new migration table

The mappings live only in the PR prose (lost after merge). Paste-ready:

| Before | After |
|---|---|
| `child: View` | `#[default] child: Child<'_>` |
| layout `slot: Result` | `slot: Slot<'_>`, interpolate `(slot)` |
| match `slot: Result` for status codes | `error_boundary` fallback |
| `#[component(boxed)]` | `ViewExt::boxed()` |
| `boundary`/`defer` skeletons | `suspense` / `live!` + `emit!` |
| bare `-> Result` | `-> Result<impl View>` / `Result<View>` |
| raw handler `-> ViewFuture<'_>` | sync `-> BoxView<'_>` via `ThenView` / `AsyncIntoResponse` |

### 7 · Docs — State the borrowing rule for views

Views borrow the `Cx` they were built against; therefore a view can never borrow anything owned by the code that returns it. Interpolating a sub-view built from a handler local (`TablePage`, `Vec<NavigationItem>`, form maps) fails with `cannot return value referencing local variable` — every consumer rediscovers this via E0515.

**Fix:** one paragraph in `view.md` — "templates may borrow `cx` freely; anything else they capture must be owned or borrowed from `cx`" — plus a worked example.

---

## Praise

- Lazy `View` + streaming is the right call, and the "no client library" promise holds: a list page shipped shell + skeleton as first content and swapped in loaded rows via `suspense` with a plain boxed future as child.
- `emit!` returning `Result<EmitToken>` (at-least-once emission enforced by types) and rethrowable `error_boundary` fallbacks are elegant.
- The migrated `topcoat-ui-registry` synced into a consumer verbatim and compiled untouched.
