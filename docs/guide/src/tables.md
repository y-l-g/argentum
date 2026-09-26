# Tables

The list view: columns and the row key, search and sort, filters, grouping and CSV export, live
updates, and row and bulk delete.

Minimal table:

```rust
Table::r#for(cx)
    .id(|u: &User| u.id.to_string())
    .pk(|u: &User| u.id.to_string())
    .columns((
        TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
            .searchable()
            .sortable(),
        TextColumn::computed("Status", |u: &User| {
            if u.active { "Active".into() } else { "Inactive".into() }
        }),
    ))
    .paginate(20)
```

Notes:

- `.id(...)` is required. It keys rows for selection and live updates. Never use a loop index.
- `.pk(...)` declares the record key that action URLs and bulk checkbox values carry; handlers
  resolve it as the model's typed primary key. Emit the primary key, not a display label. Declare it
  whenever the resource carries action chrome — `deletable()`, `editable()`, or a declared detail
  `view()` — because `Panel::build` refuses a table that carries chrome without a record key. The two projections agree in the common case
  (`|u| u.id.to_string()`).
- `searchable()` searches with `?q=`: an escaped substring match (`like_with_escape`, OR across
  searchable columns), so a term containing `%` or `_` matches those characters literally. `LIKE` is
  ASCII-case-insensitive on SQLite and case-sensitive on PostgreSQL. `sortable()` sorts with
  `?sort=` and `?dir=`. Both work without JS.
- The URL is the state: `?q=`, `?sort=`, `?dir=`, `?after=`, `?before=`, `?filters=`, `?group_by=`
  parse into `TableState`. Pagination is cursor based; Toasty appends the PK tie-breaker internally
  so cursors stay deterministic.
- Columns render in a fixed layout (`table-fixed`): a column's width is the one its header declares, not
  the widest cell on the current page, so filtering, sorting or paging never re-measures the columns.
  Widths are percentages of the table, so what a table declares is a share of its container rather than a
  length that can outgrow it. The default follows the column's kind: a field column
  (`TextColumn::r#for`) declares nothing and takes what the declared columns leave, a computed column
  (`TextColumn::computed`) claims a share (10% nominally). The chrome columns — bulk selection, row
  actions — claim shares too, and the kind defaults scale down together when their total would leave the
  field columns less than 40% of the table. `TextColumn::width(ColumnWidth::..)` overrides either; an
  explicit `Rem` does not shrink with the table, so a table narrower than its lengths leaves the field
  columns no space at all. The width is emitted as an inline `style` — Tailwind generates only the class
  literals it finds in source — and a value wider than its column truncates with an ellipsis.
- A `w-full` table never exceeds its container on its own, so on a narrow viewport the percentages
  would crush the cells instead of scrolling: the table carries a `min-width` summing its declared
  widths (shares as emitted, lengths verbatim, one readability floor per wide column, a content floor on
  the actions column), and the wrapper's `overflow-x-auto` scrolls once the table is wider than its
  container. The actions column pairs its share with that floor on its header and cells, so the row
  buttons fit instead of spilling past the table.
- Computed columns render only. They do not affect search or sort.

Filters:

```rust
.filters((
    SelectFilter::r#for(Post::fields().status(), vec!["draft".into(), "published".into()]),
    TernaryFilter::r#for(Post::fields().featured()),
    DateFilter::r#for(Post::fields().created_at()),
))
```

Active filters travel in `?filters=` and combine with AND. Unknown keys and rejected values never
fail silently: the list renders a `role=alert` banner (`Table::unapplied_filters`) while export
refuses with 400.

Grouping and export:

```rust
.group_by("status", |p: &Post| p.status.clone())
```

- Grouping is page-local with a row count per group. Toasty has no `GROUP BY` yet, so grouping never
  claims full-table totals. Unknown `?group_by=` values render no headers and drop from nav links.
- `GET /admin/{slug}/export` returns the filtered set as CSV (`text/csv; charset=utf-8` +
  `Content-Disposition`, RFC4180 with OWASP formula-defusing), reusing the same filters and sort over
  `export_query` — the base `query` unless the resource narrows it to the includes its columns
  declared (GH #177). Capped at 10k viewable rows: per-row `can_view` runs before the cap, so 413
  reflects what the caller may receive. An export whose filtered set runs past the 10,001-row scan
  window is a 413 too, even when fewer rows would be viewable: the export never returns a partial
  file. `?bom=1` opts into an Excel BOM.
- Failed table loads render the branded `ErrorState` in-region, not a blank page.

Live updates:

```rust
Table::r#for(cx).live_search(true)
```

Search, sort, filter, and pager controls then refresh the table in place without a full page load.
The plain links and forms stay as the no-JS fallback.

Panel wires the bulk checkbox column when the resource opts in with `deletable() -> true` (GH #226:
chrome is opt-in, and the flag pairs with `can_view` + `can_delete`). The column then follows those
predicates per record (GH #235): a row either one refuses renders its checkbox `disabled`
with the reason as its accessible label, so select-all never submits a key the handler would refuse
the whole batch over. A row refused every action keeps its actions cell with a `Locked` badge in place
of the links — carrying the same reason as its tooltip — so the row reads as locked rather than as
missing chrome, and the row keeps a cell per header. Bulk delete asks first: the bulk bar's
button opens an alert dialog that names how many rows are selected, and its confirm control is the
only thing carrying the `confirm=1` the handler requires — a POST without that marker is a 400, so
the safeguard does not depend on the script that opens the dialog (GH #184).

Row delete asks first too, and the dialog opens in place (GH #233): the row control names the
table's one dialog and carries that record's POST target. Confirming it needs no navigation
(GH #234): the client follows the POST's 303, mounts the flash toast the handler set, and refreshes
the table through the shard — a delete costs the confirmed POST, the list render behind the redirect
(whose body is discarded except the toast) and one `table_search` request. Cancel is a button, so
dismissing never navigates; the control's `?delete=<key>` href stays as the no-JS fallback, which
renders the same dialog open with the action already set.
