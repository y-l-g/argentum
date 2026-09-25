# Reactivity: suspense, morphing reruns, and the live-search shard

Date: 2026-08-19 — Status: accepted — Amended: 2026-09-10, 2026-09-22

## Decision

Tablo's reactivity is committed behind owned APIs: page state (query/sort/page) is owned by the
page, and resources never write their own `#[shard]` — a shard endpoint stays an optimization, not the
API surface. The migration to the current Topcoat runtime is done: `suspense` streams the resource list
behind `Table::render_skeleton`, and later reruns (page or shard) morph in place (topcoat #392) so
focus, scroll, and typing survive; reorderable rows need stable `id`s. Shards take `Signal<T>`
params (#393), and the keystroke-live, slug-dispatched `table_search` shard sits behind
`Table::live_search`, written by search, sort, filters, and pagination. The table always renders
inside a `data-boundary` region.

The `table_search` shard's wire arguments stay **scalar** — one named parameter per interaction
dimension (`path`, then `q`, `filters`, `sort`, `dir`, `cursor`, `group_by`, `bulk`) — and the
`allow(too_many_arguments)` on the shard module stays with it. This is a decision, not a limitation
of the runtime, and it was re-derived once already: a **struct** cannot travel as a shard argument
at all (topcoat requires the `expr!` vocabulary, and a struct has no `Surrogated` surrogate the
browser's `cx.hydrate` tag set can rebuild), but a **list** can — `Vec<Signal<String>>` and
`[Signal<String>; N]` are vocabulary types that round-trip through `hydrate`/`dehydrate` unchanged.
Packing the dimensions into one list argument is refused because it would couple the browser to this
server's ordering: adding or reordering a dimension silently mismatches the two halves, where a
named parameter fails to compile instead. Everything *below* the shard takes one `TableSearchArgs`
value, so a new dimension never changes a signature there. The seam's state↔signal conversions are
likewise one method each way (`TableState::to_signals`, `TableSignals::to_state`), and a request
normalizes the parsed state exactly once (`NormalizedState`, built by `Table::normalize_state`).
