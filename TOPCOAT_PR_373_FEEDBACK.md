# Review feedback — tokio-rs/topcoat PR #373

**PR:** [`feat!: streaming SSR and live! + emit! regions`](https://github.com/tokio-rs/topcoat/pull/373) (`view-stream`, head `30d92c99`).
Found exercising the branch from a real consumer (admin framework on `view` + `router` + `ui-registry`). Standalone issues only — problems someone writing fresh code against the new API hits today. Breaking changes are expected and not flagged.

| # | Type | Summary |
|---|------|---------|
| 1 | API | Opaque `impl View` can't interpolate — blanket `NodeClassify for T: View` |
| 2 | API | Raw `PageFn` needs `internal::ThenView` for fallible pages |
| 3 | API | No idiomatic list-of-views |
| 4 | Docs | State the borrowing rule: views may borrow `cx` only |

**Bugs found: 0.** Streaming, `suspense`, `live!`/`emit!`, error→status mapping and mid-stream redirects all behaved as documented.

---

### 1 · API — Blanket `NodeClassify for T: View`

`(expr)` interpolation only works for 5 hard-coded types (`Child`, `BoxView`, `ScopeView`, `MoveView`, `LiveView`). A helper returning `Result<impl View>` produces an opaque that implements `View` but not `NodeClassify`, so it can't be interpolated — every helper must return `BoxView` or be boxed at the call site just to render. The error names the wrong trait (`NodeViewParts not implemented for impl View`) and suggests nothing.

**Fix:** `impl<T: View> NodeClassify for T`, or a diagnostic saying "box it with `.boxed()`".

### 2 · API — Raw `PageFn` can't express a fallible async page with public types

`PageRenderFn` is sync: `fn(&Cx, Body) -> BoxView<'a>`. `#[page]` adapts async handlers via `topcoat_view::internal::ThenView` (a future-of-a-view becomes a view; its `Err` becomes the view's error). Hand-registering pages — the natural pattern for generic registries — has no public way to do the same: `ThenView` is `internal`, nothing else public converts a fallible future into a view.

**Fix:** public constructor (`page_from_fn(.., |cx, body| async { Result<impl View> })`) or `pub use ThenView`.

### 3 · API — No idiomatic list-of-views

Rendering a dynamic list of composed views has no first-class shape: `BoxView` is the only container and is documented only as a fix for multiple `return` sites; `Child` has no `FromIterator`; nothing documents whether `Vec<BoxView>` can be interpolated.

**Fix (any one):** `impl FromIterator<BoxView<'a>> for Child<'a>`, a `fragment!` helper, or a `view.md` note that the intended pattern is a `for` loop inside one `view!` (with `key:` on component calls).

### 4 · Docs — State the borrowing rule for views

Views borrow the `Cx` they were built against; therefore a view can never borrow anything owned by the code that returns it. Capturing a handler local (`TablePage`, `Vec<NavigationItem>`, form maps) in a template fails with `cannot return value referencing local variable` — a permanent rule of the lazy model, currently discoverable only by fighting E0515.

**Fix:** one paragraph in `view.md` — "templates may borrow `cx` freely; anything else they capture must be owned or borrowed from `cx`" — plus a worked example.

---

## Praise

- The "no client library" promise holds: a list page shipped shell + skeleton as first content and swapped in loaded rows via `suspense` with a plain boxed future as child. The swap envelope (`<template data-topcoat-swap>` + one `topcoat.swap(n)` call) is simple enough to reason about from `content/view.rs` alone.
- `emit!` returning `Result<EmitToken>` (at-least-once emission enforced by types) and rethrowable `error_boundary` fallbacks are elegant.
- The migrated `topcoat-ui-registry` synced into a consumer verbatim and compiled untouched.
