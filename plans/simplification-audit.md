# Simplification audit

What could be smaller, plainer, or delegated to a crate, with enough evidence to act on each
item. This is a coverage audit, not a correctness audit: the security and behaviour findings are
separate and already fixed or tracked.

Read against `master` at `62d47d02`. Every count is measured from the working tree (`wc -l`,
scripted greps, `diff`); estimates are marked as such. `.worktrees/` and `target/` are excluded.

## 1. Where the mass is

| Area | Lines | Notes |
| --- | --- | --- |
| First-party Rust source (excludes inline tests) | 23,569 | 64 files |
| Inline `#[cfg(test)] mod tests` inside those files | 18,041 | 55 modules |
| Integration tests (`**/tests/`) | 14,187 | 37 files, 4 test targets (2 consolidated) |
| Vendored primitives (`tablo-ui/src/components/primitives/`) | 3,870 | 31 files, synced verbatim |
| Shipped JavaScript | 1,697 | 11 assets |
| JavaScript tests | 1,755 | 6 suites |
| Documentation (Markdown) | 3,326 | 46 files |

Three ratios frame the rest. First-party Rust carries **1.37 test lines per production line**
(32,228 / 23,569). Comments are **11,576 lines**, and **1,613** of those carry a `GH #NNN`
reference. In `resource/` and `schema/` together, comments are **26%** of non-blank lines.

## 2. Findings

Severity is the value of doing it, not the size. `lines` is the estimated deletion. "Verified"
means a line-level check in this audit; "reported" means a delegated pass with spot-checks —
confirm before deleting.

### A. Dead code

**A1 — 14 of 31 vendored primitives are referenced nowhere. `verified`. (1,361 lines)**

`xtask sync-topcoat-ui` copies every component in `topcoat-ui-registry` (xtask/src/lib.rs:120),
and `primitives/mod.rs` declares all 31. These fourteen are never called:

```
accordion, avatar, badge, breadcrumb, dropdown_menu, hover_card, kbd, progress,
radio_group, spinner, switch, tabs, toggle, tooltip
```

`grep -rn "\b<name>("` over `crates/tablo-core/src`, `crates/tablo-ui/src`, and
`examples/showcase/src` returns zero hits for each (the two `toggle(` hits are
`sidebar_open.toggle()`). `lib.rs:28-64` re-exports a curated subset that excludes all fourteen.

One of them is worth calling out: `tablo-core` has its own `Tabs` schema layout
(`schema/layouts.rs`) that renders tab markup directly, while the vendored `tabs` primitive is
never called. Two tab implementations exist and only one is used.

Change: teach `xtask` a subset manifest — the registry names the app wants — so `sync` writes and
`verify` expects only those. `prune_orphans` already deletes files the registry dropped; this is
the same guard driven by an app-owned list. Do **not** hand-delete: `verify_sync` fails on an
orphan.

**A2 — Four rendered hooks have no consumer. `verified`. (~10 lines, one stale comment)**

| Hook | Rendered at | Read by |
| --- | --- | --- |
| `data-bulk-ids` | render.rs:572, 575 | nothing; bulk.js:5 documents `input[data-bulk-ids]` but the code queries `input[name="ids"]` (bulk.js:38) |
| `data-options-overflow` | select.rs:562, 574 | nothing; selects.js:19 mentions it in a comment only |
| `data-options-hint` | select.rs:624 | nothing (the hint div is unconditionally visible) |
| `data-row-delete-dialog` | render.rs:899 | tests only (delete_check.rs:58, 62, 83, 93, 130) |

Delete the attributes and the local variables behind them, fix the bulk.js comment, and move the
two test assertions onto a live hook. `ASSET_HOOKS` checks only "hook in JS ⇒ hook in Rust", so
the inverse direction is unguarded by construction (xtask/src/lib.rs:378-389) — that blind spot
is why these survived. Relatedly, `ASSET_HOOKS` omits hooks the JS **does** read:
`data-options-field`, `data-options-server`, `data-options-combobox`, `data-options-list`.

**A3 — `Panel::dark_mode` is public, documented, and untested. `verified`. (~12 lines)**

`mod.rs:392-396` sets `dark_mode: Option<bool>`, consumed at `mod.rs:591`. No call site in
`crates/`, `examples/`, or `benchmarks/`; `docs/guide/src/panel-and-routing.md:28` documents it;
`examples/showcase/src/app.rs:1566` records that the showcase deliberately omits it. Either add
the one test that pins the rendered `dark` class, or drop the method and its field. Documented
API, so this is a product decision.

**A4 — Test-only instrumentation inside production. `verified`. (~30 lines)**

`TableState::filters_param` (state.rs:320) bumps a `#[cfg(test)]` thread-local on every call
(state.rs:321-322); `filters_param_encodes`/`reset_filters_param_encodes` (state.rs:887-895)
exist only for `render.rs:3400 table_render_encodes_the_filter_transport_once_per_page_not_per_row`;
`resource/mod.rs:41-42` re-exports them `#[cfg(test)]` into the module's public path.

Pin the property the test wants directly — the encoded `filters=` value is byte-identical across
rows — and delete the counter and both re-exports. This is the only test-driven state in
production code.

**A5 — Small dead surface. `verified` / `reported`. (~35 lines)**

- `Panel::prefix()` (mod.rs:165-168) is called only by its own assertions (mod.rs:980-984).
- `FrameAncestors::same_origin` (headers.rs:60-64) is `#[cfg(test)]` with one test caller.
- `render_live_invocation`'s `_state` parameter (render.rs:1203) is unused, kept "so the seam
  keeps its shape for a future grouping control". Dead by construction; re-add with the control.
- `MAX_RELATIONSHIP_OPTIONS` (relationship.rs:253) is `pub` but referenced only inside the crate.
- The two 12-line `xtask/tests/*.rs` targets each link the whole `xtask` graph; one
  `xtask/tests/it.rs` removes a link.

**Checked clean:** a scripted reference scan found **no** unreferenced `pub fn`, `struct`, `enum`,
`trait`, `const`, or `type` anywhere in the workspace (the single hit, `embedded_form`, is the
derive entry point invoked by name). No `#[allow(dead_code)]`, no `unsafe`, no
`TODO`/`FIXME`/`HACK`. The earlier dead-surface passes (GH #204, GH #220) did the obvious work.

### B. Tests

The suite is not too big for what it protects, but it is written three times over: shared
scaffolding exists for the showcase and not for `tablo-core`, the panel's inline test modules
rebuild the same fixture per test, and several integration modules restate a unit test that
landed later.

**B1 — The core crates have no shared test harness. `reported`, spot-checked. (~270 lines)**

Each core integration module re-declares a private harness:

| Module | Re-declares |
| --- | --- |
| `uploads.rs:191-369` | `seeded_db`, `router`, `multipart_body`, `post`, `post_multipart`, `get`, `body_string`, `new_csrf`, `csp` |
| `auth_override.rs:108-227` | `seeded_db`, `router`, `post_form`, `cookies`, `login_session`, `input_value` |
| `after_commit.rs:326-393` | `seeded_db`, `router`, `post`, `seed_note`, `notes` |

`multipart_body` is written three times independently. Inside `resource/` and `schema/`, `fn cx()`
is re-declared in `fields/mod.rs:89`, `layouts.rs:476`, `tree.rs:411`; `cx_with_cookie` twice
(`csrf.rs:129`, `notification.rs:349`); and the `struct User { id, name }` toasty model appears
in at least eight test modules (`column.rs:439`, `export.rs:76`, `mod.rs:844`,
`navigation.rs:165`, `state.rs`, `lenses.rs`, …).

`examples/showcase/tests/common/mod.rs` (739 lines) is the good version of exactly this
(`TestClient`, `login`, `mint_session`, `multipart_body`, `response_cookies`, `input_value`) and
is unreachable from `tablo-core`. Add `crates/tablo-core/tests/common/mod.rs` and a
`#[cfg(test)] test_support` module for the unit tests; declare the former from `it.rs` the way
the showcase does.

**B2 — Panel inline tests rebuild the same fixture per test. `verified`. (~400-500 lines)**

Measured in `crates/tablo-core/src/panel/` alone:

- **34** of 35 `hydrate_form_values` overrides in tests are the exact trait-default stub
  (`HashMap::new()`); only `forms.rs:2681` is real.
- **44** `struct Dummy` declarations (23 in `mod.rs`); 32 are the identical
  `{ #[key] #[auto] id: Uuid, name: String }` shape, most nested inside a test fn so they shadow
  the module-level one.
- **64** `fn table(cx) -> Table<Dummy>` bodies, collapsing to 15 distinct shapes; the top three
  shapes cover 23 copies.
- **64** in-memory `Db::builder()` + **52** `push_schema()` + **76** `Panel::new(...)` calls.

One `#[cfg(test)] panel::test_support` with `dummy()`, `dummy_table(cx)`, `db_with::<R>()`,
`panel_for::<R>(db)`, plus the three or four real variants. Largest single test-size win in the
crate, no coverage change.

**B3 — `embedded_lens.rs` restates `schema/lenses.rs`. `verified`. (~130 lines)**

`crates/tablo-core/tests/embedded_lens.rs` (302 lines, 10 tests) predates
`lenses.rs:786-1240`, which covers the resolver walk directly. Eight integration tests map 1:1,
including an identical test name in both files
(`a_document_leaf_resolves_to_the_document_column`: embedded_lens.rs:173 and lenses.rs:1038).

Keep `:255 the_flattened_name_participates_in_allow_list_and_validation` and one `r#for_context`
render smoke test. Delete the rest.

**B4 — `file_repeater_check.rs` duplicates `uploads.rs` and itself. `verified`. (~210 lines)**

- `:296` ≡ `:492` ≡ `uploads.rs:473` (untouched edit keeps the stored path).
- `:348` clear-flag test ≡ `uploads.rs:829`.
- `:74` invalid-upload test ≡ `uploads.rs:436`.
- `:252` ≡ `:115`.
- `:217 posts_create_form_is_multipart` is subsumed by `:11`.

Keep the render-shape test, one valid create, one invalid create, one keep/replace, and the two
app-only tests (`:400 multipart_body_limit_matches_urlencoded_cap`,
`:449 posts_author_select_is_searchable`).

**B5 — `list_actions_check.rs` is three copies plus a re-implemented harness. `verified`. (~125 lines, 154 → ~35)**

`:11`/`:36`/`:54` are the same "list links to create and edit" test for users, authors, and posts
— one table-driven test. `:72 posts_pagination_walks_forward_and_back` duplicates
`admin.rs:325/433/502`. `:135-153 next_link`/`previous_link`/`link_href` re-implement
`common::find_pager_href` and `common::find_href_with`, including the `&amp;` decode those
helpers already do.

**B6 — `sqlite.rs` tests Toasty. `verified`. (~96 lines)**

`sqlite.rs:33 create_and_query_users` creates rows, queries them, and asserts `#[unique]` rejects
a duplicate — upstream behaviour every other test depends on. Keep
`:96 unique_collation_matches_the_app_side_probe`, which pins a real unknown (SQLite's default
`BINARY` collation agrees with the app-side unique probe).

**B7 — Test clusters that assert one state machine through several doors. `reported`. (~460 lines)**

- `relationship.rs`: six tests each insert 200-201 rows to assert overlapping facets of
  overflow → hint → targeted check → keep value (`:1012`, `:1070`, `:1152`, `:1201`, `:1291`,
  `:1368`). Two table-driven tests cover the same ground. (~250 lines, and most of the module's
  runtime.)
- `table/render.rs`: `:2295`, `:2350`, `:2388`, `:2443` all use the same `declared_percents`
  helper and the same fixtures to assert column widths. (~120 lines.)
- `resource/state.rs`: `:1287` plus six `projection_*` tests are one `project_url` call each.
  (~90 lines.)

**B8 — Tests re-assert literals another layer already owns. `reported`, spot-checked. (~200 lines)**

Eight showcase tests carry a "core owns this; this pins the HTTP wiring" comment and then
re-assert the literal anyway. `edit_check.rs:637` and `variant_check.rs:161` both assert
`` "`twelve` is not a valid whole number" ``, pinned in `typed_leaves.rs:71/160`;
`create_check.rs:340` and `tenancy_check.rs:605` assert `"has already been taken"`, pinned in
`panel/forms.rs`; `admin.rs:110` re-asserts the CSP directives that `panel/mod.rs:1361` and
`headers.rs:238` own. Assert the wiring fact (status, redirect, route) and let the core test own
the wording.

**B9 — CSRF and tenancy coverage is scattered into near-copies. `reported`. (~200 lines)**

`gate_matrix_check.rs:35 forged_posts_answer_403_and_change_nothing` already covers delete, bulk
delete, and multipart create/edit with both a mismatched and a missing token;
`create_check.rs:273`, `edit_check.rs:131`, `delete_check.rs:238`, and `bulk_check.rs:482` are
the same claim one route at a time. `tenancy_check.rs:343/376/406/458` are four near-copies of
"comments scoped through the parent post" over list, search, export, and query. Table-drive both;
the gate matrix is the destination, not a casualty.

**B10 — JS: two exact duplicates and five hand-rolled DOM stand-ins. `reported`. (~165 lines)**

- `selects.test.js:237` re-asserts exactly what `:203` asserts; `bulk.test.js:94` is subsumed by
  `:49`.
- `selects.test.js` (~150 lines of stand-in), `mutation-submit.test.js:219`, `dialog.test.js:86`
  and `:109`, and `media.test.js:62` each build a bespoke minimal DOM. A shared
  `assets/test-dom.js` removes roughly 150 of the ~400 lines with no coverage change.
- Coverage is uneven: `variant.js` (58 lines, a real hidden-field validation path) has no JS
  test, while `dialog.js` (127 lines) has 337 lines of tests for one exported function.

**Verdict on the suite as a whole:** the *binaries* are already consolidated (ADR-0015) — do not
revisit that. The cost is duplication. The "definitely redundant" items above are ~850 lines and
~30 tests; the "possibly redundant" tier is another ~800 lines and ~20 tests. Nothing in the
security, cursor-correctness, `after_commit`, upload-precedence, or tenancy suites should go.

### C. Comments and prose

**C1 — 11,576 comment lines, 1,613 carrying an issue number. `verified`.**

`GH #` citations per file: `table/render.rs` 129, `panel/forms.rs` 126, `table/mod.rs` 96,
`showcase/app.rs` 92, `resource/state.rs` 91, `panel/actions.rs` 89. Roughly **200 lines are
attribution with no explanation** — a citation appended to a sentence that stands without it, or
a citation as the whole comment. `PROSE.md:37-43` already says a comment earns its place by
explaining WHY; nothing enforces it.

This is the largest readability item and the cheapest to start. Drop the citation and keep the
sentence where the sentence carries information; delete the comment where it does not. It is a
judgement call per comment, not a lint.

**C2 — Historical narration, contrary to `PROSE.md` rule 2. `verified`. (~40 lines)**

`panel/mod.rs:909-918` ("The **old** `list_url_for_current` sniffed…"), `panel/actions.rs:74-80`
("the order … **had before** the two seeds split"), `panel/actions.rs:266-269` ("**replaces** the
#75 item-1 loop"), `panel/forms.rs:928` and `mod.rs:90,181` ("now"/"pre-#188"),
`resource/upload.rs:81` ("exactly as they were before GH #188"), `tenancy.rs:3` ("Since GH #223"),
`schema/mod.rs:55` ("since GH #187"), `assets/selects.js:10` ("used to look inert"),
`assets/variant.js:13` ("pre-#191 behaviour"), `assets/theme.js:12-15`.

**C3 — The same explanation repeated at every call site. `verified`. (~350 lines)**

- GH #298 "the scan renders no cell, so it asks for no relation includes":
  `actions.rs:436-437, 472, 600, 1794` and `forms.rs:695` — five copies of one paragraph.
- "`validate_async` loaders run on their own handle, which would block on the pool while the tx
  holds it": `forms.rs:716, 784-786, 887-889`.
- "Transport keys never reach the record fn — see create": `forms.rs:528-534, 774-778, 942`.
- `Table::with_delete`/`with_edit`/`with_view` (`table/mod.rs:553-596`) each restate the same
  six-line "chrome is opt-in, gated per record" paragraph;
  `Resource::deletable`/`editable` (`resource/mod.rs:143-173`) restate it again.
- `Resource::query`, `query_with`, and `export_query` (`resource/mod.rs:295-415`) restate the
  same "keep the non-tenant scope, do not restate the tenant filter" contract three times,
  each 30-45 doc lines.
- `OptionSource::scoped_query` and `options_query` (`relationship.rs:66-100`) restate the GH #223
  tenant paragraph.
- The `module.exports` rationale for the Node tests appears six times (`filters.js:74-78`,
  `notifications.js:103-108`, `dialog.js:121-126`, `bulk.js:228-233`, `selects.js:477-487`,
  `mutation-submit.js:365-378`).

One canonical paragraph per invariant, linked by name, is the whole fix.

**C4 — 29 doc blocks of 25 lines or more, six of 40+. `verified`. (~150-250 lines)**

Longest: `schema/embedded.rs:1` (76 lines), `tablo-macros/src/lib.rs:8` (71),
`fields/select.rs:211` (48), `resource/mod.rs:295` (48), `schema/relationship.rs:15` (47),
`resource/mod.rs:613` (43). `resource/mod.rs` carries **537 doc-comment lines for 353 code
lines** (152%) — the `Resource` trait explains the architecture to the reader instead of stating
the contract. `panel/search.rs:121-154` is a 34-line doc on a 9-line shard function. Halving the
rationale prose per block, moving what belongs to `CONTEXT.md` or an ADR, is realistic.

**C5 — Comments and docs that contradict the code. `verified`. (~40 lines)**

- `README.md:103-131` has a "Done / Next / Non-goals" roadmap. `PROSE.md:12` says to omit planned
  work; "Done" is a history list the commit log already owns.
- `docs/adr/0014-shell-js-assets.md:11` says the assets are "~59 KB unminified, ~23 KB gzipped";
  measured from the working tree they are **68.5 KB / 26.2 KB**. The same ADR lists four Node
  suites; CI runs six (`filters.test.js` and `notifications.test.js` are in the gate).
- `panel/headers.rs:60` reads like production API but is test-only (A5).

### D. Hand-rolled code a crate already does

Each candidate already resolves in `Cargo.lock` transitively, so declaring it directly adds no
new dependency version — but it does change the workspace lock, and AGENTS.md rule 7 then
requires the same commit to sync `benchmarks/tablo/Cargo.lock`.

**D1 — Hex encoders → `hex`. `verified`. (~30 lines, three sites)**

- `cursor.rs:425-449` — `hex_encode`/`hex_decode` (the decode collects to `Vec<char>` first).
- `auth.rs:469-478` — `token_key` writes `{byte:02x}` in a loop; `TokenHash` derefs to a byte
  slice, so `hex::encode` covers it.

`hex = "0.4.3"` is in the lock.

**D2 — `decode_rfc5987` → `percent-encoding`. `verified`. (~22 lines)**

`panel/forms.rs:368-388` hand-decodes `%XX` for RFC 5987 filenames.
`percent_decode_str(encoded).decode_utf8()` has the same all-or-nothing failure the current code
documents.

**D3 — Query and path percent-encoding → `percent-encoding`. `verified`, one report disputes it. (~15 lines)**

`resource/state.rs:793-814` hand-rolls the RFC 3986 unreserved set as a byte match.
`percent-encoding` expresses that set as an `AsciiSet` (`NON_ALPHANUMERIC.remove(...)`), so the
same bytes pass through. `encode_path_segment` (state.rs:812-814) is a pure delegate and folds
with it. A delegated pass notes that `encode_filter_component`/`decode_filter_component`
(state.rs:777-790) could also move to the crate; the current decode is order-dependent (`%25`
last) but does round-trip correctly, so treat that as an optional follow-up, not a bug fix.

**D4 — Test HTML scraping → an HTML parser. `verified`, needs a dependency decision. (~150 lines)**

`examples/showcase/tests/common/mod.rs` hand-writes seven scrapers plus an entity decoder:
`input_value:498`, `row_link_key:527`, `file_input_tag:547`, `find_href_with:567`,
`find_pager_href:589`, `row_titles:618`, `row_keys:646`, `unescape_href:604`. Several carry a
comment about attribute order or quote style. `common/mod.rs` is 739 lines, most of it this, and
per-file helpers re-implement it (`delete_check.rs:440 tag_with`, `media_check.rs:105`,
`bulk_check.rs:335-377`, `list_actions_check.rs:135-153`). `tl` (small, fast) or `scraper`
collapses each to a selector and makes the assertions structural, which is what `TESTING.md:17-20`
asks for. The cost is a dev-dependency and the current "parse what the browser sends" stance.

**D5 — JS test DOM stand-ins → `linkedom`/`jsdom`. `reported`, needs a decision. (~150 lines)**

Same tradeoff on the JS side, and it breaks ADR-0014's "no `package.json`, no dependency install"
stance. A shared hand-written `test-dom.js` gets most of the saving for no policy change.

**D6 — `escape_option` → `view!`. `reported`, needs confirmation. (~18 lines)**

`panel/actions.rs:810-823` hand-escapes `& < > " '` for `<option>` markup. Rendering the option
list through `view!` uses the framework's own escaper and drops the `format!` allocation; the
handler returns `http::Response<Body>` today, so the view-to-response path needs checking first.
`html-escape` is the crate alternative.

**D7 — `pluralize` → `pluralizer`. `reported`. (~40 lines, a behaviour bet)**

`resource/naming.rs:19-84` is a 65-line pluralizer with 16 irregulars, 10 uncountables, and six
`f`-exceptions. `pluralizer` is already in the lock via `toasty-macros`, but swapping changes
edge-case output, so it needs the slug and label tests to agree first. Decide, don't assume.

**Kept deliberately (do not "fix"):** `sanitize_filename` + `is_windows_reserved_name`
(`forms.rs:318-361`) re-implement `sanitize-filename` but guard a security boundary tested at
that boundary; `is_inline` (`headers.rs:221`) and `filename_star_from_headers` (`forms.rs:252`)
hand-parse MIME headers on security paths; `fnv1a_32` (`state.rs:818`) is eight lines chosen for
cross-version stability; the cursor `Value` codec (`cursor.rs:156-423`) cannot use
`postcard`/`bincode` because `toasty_core::stmt::Value` has no serde impls; `defuse_formula` +
`escape_csv` (`table/export.rs:41-66`) cover OWASP formula defusing that no crate in the tree
provides. **Already delegated, for the record:** urlencoded bodies go through `form_urlencoded`,
multipart through topcoat's multer extractor, email through `email_address`, the flash cookie
through topcoat's `CookieStore`, password hashing through `argon2`/`password-hash`, and the CSRF
compare through `subtle::ConstantTimeEq`.

### E. Structural duplication

**E1 — The field renderers repeat the same chrome four times. `verified`. (~70 lines)**

`text_input.rs:367-421`, `textarea.rs:143-188`, `select.rs:548-651`, and
`file_upload.rs:120-208` each re-derive `has_error`, `error_text`, the `ac-field--error` class,
`error_id = format!("{name}-error")`, the `aria-required`/`aria-invalid`/`aria-describedby`
trio, and the `ui_field_error` slot with `aria-live="polite"`. Only the control differs.

One `field_chrome(label, name, required, errors, control: BoxView) -> BoxView` (or a
`FieldChrome` builder) removes the copies and leaves one accessibility contract instead of four.

**E2 — Three of the four filter-bar arms are the same control. `verified`. (~70 lines)**

`table/render.rs:1317-1447` builds the identical label + `<select data-filter-name=…>` + "All"
option + option loop for `Filter::Select`, `Filter::Ternary`, and `Filter::Variant`; only the
option list and the selected-value predicate differ. One
`filter_select(cx, label, name, options, current)` covers all three.

Also in the table renderer: `render_search_bar` (`:1081-1092`) and `render_filter_bar`
(`:1510-1521`) emit the same four hidden `sort`/`dir`/`filters`/`group_by` inputs (~15 lines),
and `render_search_bar` and `render_filter_bar` are 100-plus-line functions that are mostly
`view!` markup.

**E3 — Eighteen identical `Node`/`IntoSchema` impls. `verified`. (~90 lines)**

`schema/tree.rs:143-186` has nine identical `impl From<X> for Node { Node::X(Box::new(v)) }`, and
`tree.rs:299-361` has nine identical `impl IntoSchema for X { Schema::new(self) }` (the tenth
`IntoSchema` impl, for `Schema` itself, is an identity and stays). One
`macro_rules! schema_nodes!` over the nine types removes both blocks.

**E4 — Four hand-unrolled tuple-conversion families, with three different ceilings. `verified`. (~80-110 lines)**

| Trait | Arities | Where |
| --- | --- | --- |
| `IntoColumns` | 1..5 | column.rs:381-433 |
| `IntoFilters` | 1..4 | filter.rs:383-420 |
| `IntoSchema` | 1..4 | tree.rs:290-400 |
| `IntoRelationColumns` | recursive, any arity | relation.rs:111-121 |

`IntoRelationColumns` already uses the recursive form, so the inconsistent ceilings are an
accident rather than a constraint — the doc at column.rs:389-393 admits the 5-column limit.
`IntoColumns` is homogeneous, so a single
`impl<M, const N: usize> IntoColumns<M> for [TextColumn<M>; N]` replaces five impls **and** lifts
the limit (a users table with name, email, role, active, created_at is already at it). The mixed
traits need the recursive form or a macro; take one shape everywhere.

**E5 — Eight hand-written `Debug`/`Clone` impls. `verified`. (~60 lines)**

`filter.rs:15-34, 82-99, 145-162, 235-257`. `#[derive]` is not usable because it would add an
`M: Clone + Debug` bound that `toasty::schema::Model` does not carry, but `Path`'s `Clone` and
`Debug` are unconditional at the locked toasty rev, so a `macro_rules!` generator is safe and
bound-free.

**E6 — The cursor codec's two 20-arm matches. `verified`. (~100 lines)**

`write_value` (`cursor.rs:156-252`) and `read_value_with_depth` (`:256-378`) match the same 20 tag
constants (`:27-47`) in the same order. A macro keyed on `(tag, Variant, Type, to_le_bytes)` would
emit both and remove the class of bug where a tag reaches one side only.

**E7 — Thin twins and duplicated blocks. `verified`. (~150 lines)**

- `find_by_key`/`find_by_key_narrowed` (actions.rs:48-72) and
  `load_viewable`/`load_viewable_narrowed` (actions.rs:120-146) differ only in the query seed;
  each non-narrowed form has exactly one caller.
- The composite-PK error block is byte-identical at actions.rs:89-98 and 272-281.
- The panel-prefix fallback is written twice: `mod.rs:920-929` and `shell.rs:436-443`.
- The list header and Create button are byte-identical at `list.rs:250-265` and `355-370`.
- The auth + tenant prologue is adjacent at 11 sites (actions.rs 163/233/448/724, detail.rs:34,
  forms.rs 474/743/850/878, list.rs:199, search.rs:71).
- `relationship.rs:289-317` and `:345-388` repeat the bounded-load tail (limit + 1, `warn`, and
  `LoadFailed`) four times in the module; one `bounded_options` helper covers it (~30 lines).
- `schema/mod.rs:301-311 has_file_upload` re-walks the tree that `file_uploads()` (`:291-296`)
  already walks via `leaves`; `!self.file_uploads().is_empty()` is the same answer (~10 lines).

**E8 — The create and edit POST pipelines are ~80% the same. `verified`. (~40-60 lines)**

`forms.rs:741-844` (104 lines) and `forms.rs:876-1001` (126 lines) share, in order: auth +
tenant, body parse, CSRF verify, unknown-key rejection, `FormParts` destructure, client-typed
upload drop, `store_uploads`, `restore_pending_uploads`, transport-key strip, `validate_async`,
upload-error merge, transaction open, `check_unique`, `rerender_invalid_form`,
`normalize_values`, commit, `run_after_commit`, `redirect_after_write`, and the failure toast and
`hook_failure` mapping. Only the advisory load and the untouched-file backfill differ. Extract
`prepare_submission` and `commit_write`; this removes a live drift risk, not just lines.

**E9 — The example restates the framework's defaults. `verified`. (~180 lines)**

`examples/showcase/src/app.rs` implements `delete_record` and `bulk_delete_records` four times
each as the same `Self::query(&cx).filter(Model::fields().id().eq(rec.id)).delete().exec(ex)`
loop (e.g. app.rs:231-274), and every `update_record` re-implements the "absent key keeps the
stored value" rule by hand (app.rs:186-229). A framework helper for the default delete and a
typed form-values accessor (`values.get_or_keep("name", &record.name)`) would cut the example
substantially — and the example is what users copy.

**E10 — `NotificationStatus::as_str` restates the serde rename. `verified`. (~7 lines)**

`notification.rs:31-49` has both `#[serde(rename_all = "lowercase")]` and a hand-written `as_str`
returning the same four strings.

### F. API surface and long functions

**F1 — Very long functions. `verified`.**

| Lines | Function | Where |
| --- | --- | --- |
| 321 | `render_inner` | table/render.rs:200-520 |
| 269 | `expand_enum` | tablo-macros/src/embedded.rs:490-768 |
| 266 | `render_filter_bar` | table/render.rs:1284-1549 |
| 219 | `render_with` | schema/fields/select.rs:436-654 |
| 212 | `Panel::build` | panel/mod.rs:446-657 |
| 187 | `Panel::render_shell_body` | panel/shell.rs:240-416 |
| 151 | `render_thead` | table/render.rs:1849-1999 |
| 145 | `resource_export` | panel/actions.rs:446-590 |
| 127 | `resource_edit_post` | panel/forms.rs:876-1001 |
| 124 | `resource_list_live` | panel/list.rs:285-408 |

`too_many_lines` is allowed at the workspace level, so nothing flags these. `render_inner` and
`Panel::build` are the two that mix unrelated phases (validate / render-part / mount); splitting
them is net-zero lines and a large cognitive win.

**F2 — `project_url` takes eight positional arguments. `verified`. (~40 lines)**

`resource/state.rs:494-529` takes `search, sort, dir, filters, group_by, after, before, delete`,
with an `#[allow(clippy::too_many_arguments)]`, and all seven callers (`:362-467`) pass the same
eight-slot list. A `struct UrlProjection<'a>` with `..Default::default()` at call sites makes
"which intent drops which parameter" readable.

**F3 — `validate_async` string-matches a generated message. `verified`. (correctness, ~8 lines)**

`schema/mod.rs:392` does `.filter(|e| !e.contains("is required"))` to undo the required check it
already ran. Rewording `required_error` (`validation.rs:193-195`) silently changes behaviour.
Have `Select` expose an existence-only check, or return a typed error kind.

**F4 — Five render entry points exist to carry a normalization proof. `verified`.**

`render`, `render_with_state`, `render_normalized`, `render_live_with_state`, and
`render_live_normalized` (table/render.rs:102, 124, 142, 167, 185) differ only in whether they
take `&TableState` or the `NormalizedState` newtype and whether they take `Option<TableSignals>`.
ADR-0003 and GH #224 defend the "normalize exactly once per request" invariant; the cost is five
methods and a `Deref` newtype for an invariant a cheaper normalization (idempotent, or a private
field set once) could hold. Worth a design pass before more code lands on top.

### G. Client JavaScript, and the htmx question

**Verdict: do not add htmx. `verified`.**

Every one of the ten scripts is load-bearing or a fallback, and none is replaceable by an htmx
attribute without a framework-level change:

- Topcoat hydrates **once** at `runtime.start(document)` and thereafter only from its own render
  units, whose insert path calls `morph` then `runtime.hydrate`. There is no `MutationObserver`
  hydrating arbitrary inserted DOM, and htmx does not run scripts in swapped content. htmx-swapped
  markup would leave every `@click`/`:value` binding and shard scope marker dead; re-running
  `runtime.start()` constructs a second runtime.
- The framework already implements htmx's core: `ShardUnit` is signal-driven fetch + morph +
  hydrate; `PageUnit` is fetch-current-URL-and-morph. Adding htmx puts two reactivity systems on
  one page.
- There is no HTML-partial endpoint to point `hx-get` at. The only table re-render is the
  `table_search` shard, a POST carrying JSON surrogates.
- `bulk.js`, `dialog.js`, `notifications.js`, `selects.js`, `theme.js`, `sidebar.js`, and
  `variant.js` have no htmx idiom: client-side selection state, a modal with focus and Escape
  handling, timers, an ARIA combobox, a cookie, a display toggle.
- `hx-boost` removes none of them (none is a navigation) and would break suspense and shards by
  swapping the body without hydrating.
- Cost: +52 KB raw / +17 KB gzipped **on every document, including login**, against 68.5 / 26.2 KB
  for all ten scripts today. The best-case deletion is `filters.js` + `live-search.js` (152 lines)
  only after adding a partial route that re-renders what the shard already renders, and it
  regresses morph, focus, and abort coalescing.

If htmx is ever wanted, the honest framing is "replace `suspense` + shards", which is a framework
project, not cleanup. (The pinned topcoat also ships an unused `topcoat-datastar`; the reactivity
question is upstream's to answer.)

**The JS wins that are real:**

- G1: the four dead hooks and one stale comment (A2).
- G2: `wireOf`/`wireFrom` are byte-identical in `bulk.js:42-51` and `mutation-submit.js:73-82`.
- G3: `variant.js:51-55` branches on `document.readyState === 'loading'`, unreachable under the
  all-`defer` policy (ADR-0014). `filters.js:58-62`'s `requestSubmit` fallback is dead in target
  browsers. `sidebar.js`'s Ctrl/Cmd+B is 9 of its 38 lines.
- G4: comment density is 54% in `dialog.js`, 50% in `variant.js`, 37% in `bulk.js` and
  `mutation-submit.js` — the same trim as C1-C4.
- G5: `theme.js` (19 lines, 15 code) could fold into the already-inline `theme_init_script`
  (`composites/theme.rs:57-61`), removing an asset and a `lib.rs` constant.
- G6: the all-load policy ships all ten scripts on every document, including the login page where
  seven are no-ops. ADR-0014 says to revisit "if they grow"; correcting the ADR's stated size is
  the minimum, and a per-page asset set is the larger option.

Realistic JS reduction with no behaviour change: **~120-150 lines of 1,697**.

### H. Tooling and repo

**H1 — The `xtask` hook contract is one-directional and large. `reported`.**

`ASSET_HOOKS` (xtask/src/lib.rs:390-607) lists 38 needles and `verify_asset_hooks`
(lib.rs:719-791) checks each appears in the JS and in comment/test-stripped Rust. It catches a
renamed hook and misses both a rendered hook nobody reads (A2) and a hook the JS reads but the
table omits. The asset-exists and constant-still-declared half is genuinely valuable (`asset!`
does not stat its source). Options: keep it, extend it to the inverse direction, or move the hook
list into a header comment in each asset so one edit covers both sides.

**H2 — Two compile-only benchmark stubs. `verified`, no action.** `benchmarks/axum-maud` (21
lines) and `benchmarks/leptos` (~56 lines) are documented as non-comparable smoke stubs. They
cost one `cargo fmt --check` each and nothing else; proportionate.

**H3 — CI names the JS suites by hand. `verified`, deliberate.** `.github/workflows/ci.yml:30-40`
lists six files rather than globbing, so a renamed suite fails the job instead of silently
shrinking the run. The reasoning is in the `check` skill. Leave it.

## 3. Explicitly rejected

| Idea | Why not |
| --- | --- |
| Replace the custom JS with htmx | See G. Net addition, no partial endpoint, breaks hydration. |
| Re-split the test binaries / undo ADR-0015 | Already consolidated; the remaining test cost is duplication, not linking. |
| Rename `*_check.rs` to match `admin.rs` | Cosmetic churn with no reader benefit. |
| Blanket `cargo update` | `topcoat`/`toasty` track `main` and are bumped deliberately (AGENTS.md). |
| Delete `sanitize_filename`, `is_inline`, `filename_star_from_headers`, the cursor codec, or CSV defusing | Security boundaries or gaps no crate in the tree covers. |
| Delete the security, cursor, `after_commit`, upload-precedence, or tenancy tests | Verified as protecting real behaviour; the audit's target is the duplication around them. |
| Add an HTML parser to the Rust tests without deciding the stance first | D4/D5 are decisions about test philosophy, not mechanical cleanup. |

## 4. Execution plan

Six batches. Each is independently mergeable and ends with the gate set for what it touched plus
`cargo test --workspace --locked` (AGENTS.md rule 3). Batches 1-3 change no behaviour; 4-6 are
reviewed like features.

**Batch 1 — dead code (A, G1-G3).** Delete the four dead hooks and fix the stale comment; decide
`Panel::dark_mode` (test or drop); replace the `filters_param_encodes` counter with a structural
assertion; resolve `Panel::prefix`, `same_origin`, `_state`, `MAX_RELATIONSHIP_OPTIONS`, and the
two xtask targets; merge the duplicate JS wire helpers; drop the unreachable `variant.js` branch.
`~100 lines`.
Gates: `cargo test --workspace --locked`, clippy, `node --test`, `cargo test -p xtask`.

**Batch 2 — test duplication (B).** Add the core `tests/common/mod.rs` and the panel
`test_support` module (B1, B2) **first**, then the deletions (B3-B6, B10). Do a coverage pass
before touching the B7-B9 "possibly redundant" tier; do not delete those on reading alone.
`~1,450 lines` for B1-B6 and B10, up to ~2,300 with that tier.
Gates: the full ten minus the JS suites, if JS is untouched.

**Batch 3 — prose (C).** Strip attribution-only comments and historical narration (C1, C2),
de-duplicate the repeated explanations and contract paragraphs (C3), halve the long doc blocks
(C4), fix the ADR and README drift (C5). Reviewable only if it lands per-file, so split by crate.
`~500-700 lines`.
Gates: `RUSTDOCFLAGS=-D warnings cargo doc`, `mdbook build docs/guide`; `PROSE.md` is the rubric.

**Batch 4 — crate swaps (D).** `hex` (two sites) and `percent-encoding` (two sites), then decide
D4 and D5. `~70 lines` for D1-D3; ~300 with the test-parser decision.
Gates: the full ten, plus AGENTS.md rule 7's `benchmarks/tablo/Cargo.lock` sync if the
workspace lock changes.

**Batch 5 — structural (E1-E3, E5, E6, E7, E8, E10, F2, F3).** The local, high-confidence ones
first: field chrome, filter select, the schema macro, the `Debug`/`Clone` macro, the cursor macro,
the twins, the create/edit pipeline. Then the `project_url` struct and the `validate_async`
typing. `~500 lines`, but the value is drift removal and one accessibility contract.
Gates: the full ten.

**Batch 6 — API and tooling decisions (A1, E4, F1, F4, H1).** Each has a breaking edge, so each
gets its own issue and a short `docs/dev/design/` note before code:
- the `xtask` subset manifest that stops vendoring 14 unused primitives (A1);
- one tuple-conversion shape and the arity ceilings (E4);
- splitting `render_inner` / `render_filter_bar` / `Panel::build` (F1);
- whether to collapse the five render entry points (F4);
- whether to invert or keep the hook contract (H1).

## 5. Needs measurement before acting

1. **Coverage run** (`cargo llvm-cov` or a mutation run) before deleting the B7-B9 "possibly
   redundant" tier — those rest on reading and in-file comments, not on execution.
2. **`xtask` subset manifest feasibility**: whether `topcoat-ui-registry` exposes enough for
   `verify_sync` to expect a subset without false drift (A1).
3. **`escape_option` → `view!`**: whether `resource_options` can return a rendered view rather
   than `http::Response<Body>` (D6).
4. **Compile-time effect** of the 14 dead primitives and the large inline test modules —
   `cargo build --timings` before and after, so the A1/B2 payoffs are stated in seconds as well
   as lines.
5. **`pluralizer` edge cases** (D7) — compare its output against `naming.rs`'s tests before
   swapping.

## 6. What this audit checked and found clean

- **No dead Rust public surface.** Every `pub fn`/`struct`/`enum`/`trait`/`const`/`type` has a
  reference (one derive entry point excepted).
- **No suppressed lints or markers**: no `#[allow(dead_code)]`, no `unsafe`, no
  `TODO`/`FIXME`/`HACK`. The two `#[allow(clippy::too_many_arguments)]` are both documented: the
  shard's scalar wire contract (`panel/search.rs:155`, which the module doc at `:141` explains)
  and `project_url` (`state.rs:493`, which F2 removes).
- **Already delegated**: URL-encoded bodies (`form_urlencoded`), multipart (topcoat's multer
  extractor), email (`email_address`), flash cookies (topcoat `CookieStore` + serde), password
  hashing (`argon2`/`password-hash`), CSRF compare (`subtle::ConstantTimeEq`), kebab-case
  (`heck`).
- **Small and justified**: `panel/headers.rs`, `panel/detail.rs` production, the `search.rs` shard
  wiring, `tenancy.rs` (143 code lines), `schema/pk.rs`, `resource/commit.rs`,
  `resource/navigation.rs`, `table/export.rs`.
- **Contracts that are dense but right**: the cursor codec, the policies, the tenancy filters, the
  upload precedence rules, the `after_commit` seam, and the relationship option cap. Their tests
  are valuable.
- **Test binaries** are already consolidated; do not revisit ADR-0015.
</content>
