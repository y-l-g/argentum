# Implementation Plans

Generated on 2026-09-24 against commit `cbb738a8` from a full audit of the workspace. All six plans
are executed; each row below records the pull request and the squash commit on `master`.

## Execution order & status

| Plan | Title | Priority | Effort | Depends on | Status |
|------|-------|----------|--------|------------|--------|
| [001](001-fileupload-stored-value-xss.md) | Close the stored-XSS path through `FileUpload` values | P1 | M | — | DONE (#284, `87b3fbd2`) |
| [002](002-serve-dir-inert-content.md) | Serve uploaded files as inert content from `Panel::serve_dir` | P1 | M | — | DONE (#283, `5c3436e0`) |
| [003](003-export-no-silent-truncation.md) | Refuse, never truncate, an export whose scan window is too small | P1 | S | — | DONE (#285, `98780e80`) |
| [004](004-date-filter-overflow-panic.md) | Stop a date filter on the last representable day from panicking | P1 | S | — | DONE (#286, `0e146fec`) |
| [005](005-route-gate-test-matrix.md) | Pin every route's auth, CSRF, tenant and policy gates with tests | P1 | M | — | DONE (#287, `8daf98a0`) |
| [006](006-auth-off-fails-closed.md) | Make the `auth`-off build fail closed and keep its tests compiling | P1 | M | — | DONE (#288, `8f4d3e3b`) |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (with one-line reason) | REJECTED (with one-line rationale)

## Dependency notes

- None of the six blocks another; each can run on its own branch, in its own worktree with its own
  `CARGO_TARGET_DIR` (`AGENTS.md` rule 4).
- 001 and 005 both touch `examples/showcase/tests/`: whichever merges second re-runs
  `cargo test -p showcase --test it`.
- 005 and 006: 005's tests use `.auth(crate::Auth::disabled())`, which 006 makes compile without
  the `auth` feature; whichever merges second re-runs
  `cargo test -p argentum-core --no-default-features --locked` (valid only after 006).
- 005 lands before any refactor of how panel handlers load records (finding 12 below).
- Run `cargo test --workspace --locked` on each merged result.

## Findings, ranked by leverage

Evidence locations are HEAD `cbb738a8`. ✔ marks findings I reproduced or traced to the line myself.

| #   | Finding                                                                                                                                                                                                                                                                                                               | Category       | Impact  | Effort | Risk | Evidence                                                                                                      |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- | ------- | ------ | ---- | ------------------------------------------------------------------------------------------------------------- |
| 1   | ✔ **Stored XSS through `FileUpload`:** a plain text value (a url-encoded field or a multipart part with no filename) never goes through the uploader, is saved as is, and is rendered unchecked as `href`. A user with create or update rights can store a `javascript:` link that runs when another admin clicks it. | security       | High    | S      | Low  | `panel/forms.rs:162-168`, `upload.rs:139-142`, `schema/fields/file_upload.rs:233-272`                         |
| 2   | ✔ **Uploaded HTML or SVG files are served from the admin's own origin** with their real content type. There is no `nosniff`, no sandbox policy and no download header. The showcase uploader keeps the client's file extension.                                                                                       | security       | High    | M      | Med  | `panel/mod.rs:196-205`, topcoat `route/directory.rs:317-324`, showcase `app.rs:1430-1447`, `media.rs:361-426` |
| 3   | ✔ **The CSV export silently drops rows** when `can_view` hides rows: the scan stops after 10,001 raw rows, so for example 20k rows with half hidden gives a 200 with about 5k rows. This breaks the "never truncate silently" contract.                                                                               | bug            | High    | S      | Med  | `panel/actions.rs:405-413, 551-558`                                                                           |
| 4   | ✔ **A date filter on `9999-12-30` crashes the request** (the 24-hour addition overflows) on the list page, the live shard and the export. Any viewer can trigger it.                                                                                                                                                  | bug            | Med     | S      | Low  | `resource/filter.rs:198-201`                                                                                  |
| 5   | ✔ **`filters.js` does not escape `,`, `:` or `%`** in filter values, but the server decodes them. An option like `Smith, John` applies the wrong filter plus a warning. There is no JS test.                                                                                                                          | bug            | Med     | S      | Low  | `assets/filters.js:20-31` vs `resource/state.rs:753-789`                                                      |
| 6   | **When a form re-renders with errors, a file just uploaded shows as "Current" but is thrown away.** On create the next submit fails "required"; on edit the old file is silently kept.                                                                                                                                | bug            | Med     | M      | Med  | `upload.rs:148-151`, `forms.rs:615, 756-767`, `file_upload.rs:117,176-199`                                    |
| 7   | ✔ **The live table's Retry link does nothing** when the table is in its default state, because topcoat does not re-run anything when a signal is set to the value it already holds. In any other state it silently drops the user's search, filters and sort.                                                         | bug            | Med     | S      | Low  | `panel/list.rs:153-171`, topcoat `reactivity.ts:24`                                                           |
| 8   | ✔ **The toast explaining a failed write never shows on the 500 page**, which is topcoat's plain-text body. The toast appears later on some unrelated page, and the doc comment says the opposite.                                                                                                                     | bug            | Med     | M      | Low  | `notification.rs:138-142`, topcoat `internal_server.rs:60-63`                                                 |
| 9   | **Route × gate test matrix.** There is no CSRF-rejection test for bulk delete, login, logout or multipart create/edit. No test sends a valid-token edit or delete POST across tenants. The live-search shard's tenant and policy gates are untested.                                                                  | tests          | High    | M      | Low  | `examples/showcase/tests/*` (TEST-01/02/03)                                                                   |
| 10  | ✔ **Building without default features turns off all auth with no warning**, and the tests cannot build in that mode (about 85 errors). CI only runs `check`, so it never compiles the test code.                                                                                                                      | security/tests | Med     | M      | Low  | `panel/mod.rs:826-830`, `Cargo.toml [features]`, `ci.yml:34`                                                  |
| 11  | ✔ **The `frame-ancestors` header is missing from every error response and redirect**, because of the `?` in the header layer. The comment and the security guide say "every response".                                                                                                                                | security       | Low     | S      | Low  | `panel/headers.rs:50-59`                                                                                      |
| 12  | **Each resource's includes (`Resource::query`) reach every loader**: relationship option lists, the edit handler (twice), delete, bulk delete and the pagination probe. In the showcase, every Comment form load pulls in every comment of up to 201 posts.                                                           | perf           | High    | M      | Med  | `resource/mod.rs:741`, `schema/relationship.rs:213-225`, `actions.rs:68`, `table/mod.rs:772`                  |
| 13  | **The export reads the whole window from the database twice** (a counting pass, then a streaming pass).                                                                                                                                                                                                               | perf           | Med     | M      | Med  | `actions.rs:407-482`                                                                                          |
| 14  | ✔ **An empty export returns a 0-byte CSV** with no header and no BOM.                                                                                                                                                                                                                                                 | bug            | Low     | S      | Low  | `actions.rs:439-460, 565`                                                                                     |
| 15  | ✔ **A slug or prefix containing `{ } ( ) *` makes `Panel::resource` panic**, contrary to its docs (GH #174).                                                                                                                                                                                                          | bug            | Low     | S      | Low  | `panel/mod.rs:643-647, 808-812`                                                                               |
| 16  | ✔ **The documented examples are broken:** the README quick-start doesn't compile (no one-element-tuple `columns` impl), `detail-pages.md` calls a function that doesn't exist, and the guide never mentions `Table::pk`, so following it ends in a boot error.                                                        | docs           | Med     | S      | Low  | `README.md:40-44`, `column.rs:397-432`, `detail-pages.md:50`                                                  |
| 17  | ✔ **The benchmark's HTTP mode gets 403 on `/admin/posts`** because no tenant is injected. The benchmark README also claims `#[memoize]`, `try_join!` and a count query, none of which exist.                                                                                                                          | dx/perf        | Med     | S      | Low  | `benchmarks/argentum/src/main.rs:125,465-497`, `benchmarks/README.md:68-76`                                   |
| 18  | ✔ **Searchable-select combobox bugs:** the edit form shows an empty box, Enter submits the form while "Searching…" is showing, and after a server search the current option shows as a raw UUID. Its ARIA combobox wiring is also incomplete (M effort).                                                              | ui             | Med     | S      | Low  | `assets/selects.js:85,251-267,330`, `select.rs:585-618`                                                       |
| 19  | ✔ **Toast auto-dismiss pausing is broken:** a timer that is still running is never cleared, so a focused toast can disappear under the reader.                                                                                                                                                                        | ui             | Low     | S      | Low  | `assets/notifications.js:38-49`                                                                               |
| 20  | **EmbeddedForm derive issues:** a variant field named `spec`, `out`, `cx` or `parent` breaks compilation; the default label for a field like `r#type` renders as `R#type`; `textarea` on a non-String field gives a confusing error. There are no compile-fail tests.                                                 | bug/tests      | Low     | S–M    | Low  | `argentum-macros/src/embedded.rs:145-161, 224-230, 572-577`                                                   |
| 21  | **Validation edge cases:** fields in a hidden variant group are still validated; `f32`/`f64` fields accept `NaN` and `inf`; select values are checked after trimming but written untrimmed; `typed().unique()` checks with a String binding.                                                                          | bug            | Low–Med | S each | Low  | `tree.rs:130-141`, `validation.rs:50-85`, `schema/mod.rs:130-141`, `text_input.rs:145-220`                    |
| 22  | **The detail page's relation table** ignores the related resource's `can_view` and has no row limit.                                                                                                                                                                                                                  | security/perf  | Low–Med | S      | Low  | `resource/relation.rs:116-140`, `panel/detail.rs:49`                                                          |
| 23  | **Tooling drift:** the Renovate `syn <3` rule is stale; Renovate's lock-file maintenance breaks the lockfile lockstep rule; there is no `cargo deny`/`cargo audit` step; the ten documented gates are not what CI runs; new JS test suites are not picked up automatically; nightly is not pinned.                    | dx/deps        | Low–Med | S      | Low  | `renovate.json:8-17`, `ci.yml`, `AGENTS.md`, `CONTRIBUTING.md:33`                                             |

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
