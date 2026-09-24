# Audit findings

Findings from a full audit of the workspace at commit `cbb738a8`, 2026-09-24, that remain open.
Evidence locations are that commit; `✔` marks the findings reproduced or traced to the line during
the audit.

## Findings, ranked by leverage

| # | Finding | Category | Impact | Effort | Risk | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | ✔ **`filters.js` does not escape `,`, `:` or `%`** in filter values, but the server decodes them. An option like `Smith, John` applies the wrong filter plus a warning. There is no JS test. | bug | Med | S | Low | `assets/filters.js:20-31` vs `resource/state.rs:753-789` |
| 2 | **When a form re-renders with errors, a file just uploaded shows as "Current" but is thrown away.** On create the next submit fails "required"; on edit the old file is silently kept. | bug | Med | M | Med | `upload.rs:148-151`, `forms.rs:615, 756-767`, `file_upload.rs:117,176-199` |
| 3 | ✔ **The live table's Retry link does nothing** when the table is in its default state, because topcoat does not re-run anything when a signal is set to the value it already holds. In any other state it silently drops the user's search, filters and sort. | bug | Med | S | Low | `panel/list.rs:153-171`, topcoat `reactivity.ts:24` |
| 4 | ✔ **The toast explaining a failed write never shows on the 500 page**, which is topcoat's plain-text body. The toast appears later on some unrelated page, and the doc comment says the opposite. | bug | Med | M | Low | `notification.rs:138-142`, topcoat `internal_server.rs:60-63` |
| 5 | ✔ **The `frame-ancestors` header is missing from every error response and redirect**, because of the `?` in the header layer. The comment and the security guide say "every response". | security | Low | S | Low | `panel/headers.rs:50-59` |
| 6 | **Each resource's includes (`Resource::query`) reach every loader**: relationship option lists, the edit handler (twice), delete, bulk delete and the pagination probe. In the showcase, every Comment form load pulls in every comment of up to 201 posts. | perf | High | M | Med | `resource/mod.rs:741`, `schema/relationship.rs:213-225`, `actions.rs:68`, `table/mod.rs:772` |
| 7 | **The export reads the whole window from the database twice** (a counting pass, then a streaming pass). | perf | Med | M | Med | `actions.rs:407-482` |
| 8 | ✔ **An empty export returns a 0-byte CSV** with no header and no BOM. | bug | Low | S | Low | `actions.rs:439-460, 565` |
| 9 | ✔ **A slug or prefix containing `{ } ( ) *` makes `Panel::resource` panic**, contrary to its docs (GH #174). | bug | Low | S | Low | `panel/mod.rs:643-647, 808-812` |
| 10 | ✔ **The documented examples are broken:** the README quick-start doesn't compile (no one-element-tuple `columns` impl), `detail-pages.md` calls a function that doesn't exist, and the guide never mentions `Table::pk`, so following it ends in a boot error. | docs | Med | S | Low | `README.md:40-44`, `column.rs:397-432`, `detail-pages.md:50` |
| 11 | ✔ **The benchmark's HTTP mode gets 403 on `/admin/posts`** because no tenant is injected. The benchmark README also claims `#[memoize]`, `try_join!` and a count query, none of which exist. | dx/perf | Med | S | Low | `benchmarks/argentum/src/main.rs:125,465-497`, `benchmarks/README.md:68-76` |
| 12 | ✔ **Searchable-select combobox bugs:** the edit form shows an empty box, Enter submits the form while "Searching…" is showing, and after a server search the current option shows as a raw UUID. Its ARIA combobox wiring is also incomplete (M effort). | ui | Med | S | Low | `assets/selects.js:85,251-267,330`, `select.rs:585-618` |
| 13 | ✔ **Toast auto-dismiss pausing is broken:** a timer that is still running is never cleared, so a focused toast can disappear under the reader. | ui | Low | S | Low | `assets/notifications.js:38-49` |
| 14 | **EmbeddedForm derive issues:** a variant field named `spec`, `out`, `cx` or `parent` breaks compilation; the default label for a field like `r#type` renders as `R#type`; `textarea` on a non-String field gives a confusing error. There are no compile-fail tests. | bug/tests | Low | S–M | Low | `argentum-macros/src/embedded.rs:145-161, 224-230, 572-577` |
| 15 | **Validation edge cases:** fields in a hidden variant group are still validated; `f32`/`f64` fields accept `NaN` and `inf`; select values are checked after trimming but written untrimmed; `typed().unique()` checks with a String binding. | bug | Low–Med | S each | Low | `tree.rs:130-141`, `validation.rs:50-85`, `schema/mod.rs:130-141`, `text_input.rs:145-220` |
| 16 | **The detail page's relation table** ignores the related resource's `can_view` and has no row limit. | security/perf | Low–Med | S | Low | `resource/relation.rs:116-140`, `panel/detail.rs:49` |
| 17 | **Tooling drift:** the Renovate `syn <3` rule is stale; Renovate's lock-file maintenance breaks the lockfile lockstep rule; there is no `cargo deny`/`cargo audit` step; new JS test suites are not picked up automatically; nightly is not pinned; and the ten documented gates omit the rustdoc `-D warnings` build, the guide build, and the detached-bench formatting and lockstep checks that CI runs. | dx/deps | Low–Med | S | Low | `renovate.json:8-17`, `ci.yml`, `AGENTS.md`, `CONTRIBUTING.md:33` |

**Lower-leverage findings, all small (S):**

- A cursor that decodes but is stale traps Retry in the same failure.
- A relationship option can never be picked when its label is a substring of more than 200 others.
- `aria-sort` reports "none" on the column the table is actually sorted by.
- Nothing stops two filters from sharing a name, as happens for columns.
- Three different ways of building DOM ids disagree.
- `mutation-submit.js` can re-send a delete that already committed (when `after_commit` panics).
- A race in the delete dialog, and alert dialogs close on a backdrop click.
- The login route accepts bodies up to the 10 MiB form cap.
- Expired sessions are never cleaned up.
- `schema.file_uploads()` is rebuilt for every `clear_*` key.
- `architecture.md` says `table()`/`form()` run "once at boot", but they are rebuilt on every request.
- ADR-0014 says "no Node step in CI", which is wrong, and several rustdoc comments break the "describe current behaviour only" rule.
- No tests for cursor pagination with tied sort values or descending order.
- About 30 test assertions pin exact wording, including topcoat's generated code.
