# Plan 003: Refuse, never truncate, an export whose raw scan window is too small

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- crates/argentum-core/src/panel/actions.rs docs/guide/src/tables.md docs/adr/0012-tenancy-grouping-export.md`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: MED — resources with a row-level `can_view` over large tables get a 413 where they got a (truncated) 200
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

The CSV export promises two things (rustdoc in `panel/actions.rs` and
`docs/guide/src/tables.md:70`): the 10k cap counts *viewable* rows, and a 200
CSV is never silently truncated. The implementation only ever scans a raw window
of `MAX_EXPORT_ROWS + 1` rows. When a resource hides rows with `can_view`, the
viewable count inside that window can be under the cap while more viewable rows
exist past it — the export answers 200 with a partial CSV. Example: 20,000
matching rows, half denied → ~5,000 rows exported, ~5,000 silently missing. The
fix keeps the bounded window (memory and DB bound are deliberate, ADR-0012) and
turns "window full and rows remain beyond it" into the same 413 the cap already
uses — an honest refusal instead of a quiet partial file.

## Current state

- `crates/argentum-core/src/panel/actions.rs`:
  - Constants, ~line 279-289: `MAX_EXPORT_ROWS = 10_000`, `EXPORT_CHUNK_ROWS = 500`.
  - `enforce_export_cap_count(count)` (~line 294) → 413 via
    `topcoat::router::error::content_too_large()` when `count > MAX_EXPORT_ROWS`.
  - `resource_export` (~line 382). Phase 1:

    ```rust
    // actions.rs:405-413
    let mut chunker = ExportChunker::new(export_base_query::<R>(cx, &table, &state)?);
    let mut db_handle = db(cx);
    let mut visible = 0usize;
    while let Some(rows) = chunker.next_chunk(&mut db_handle).await? {
        visible += rows.iter().filter(|r| R::can_view(cx, r)).count();
    }
    enforce_export_cap_count(visible)?;
    ```

    Phase 2 (~line 414-484) re-walks the window on a spawned task with a second
    `ExportChunker`, streaming CSV; on a mid-stream overrun it calls
    `tx.abort(std::io::Error::other("export overflowed its cap"))`.
  - `ExportChunker` (~line 532) and its walk:

    ```rust
    // actions.rs:551-581
    async fn next_chunk(&mut self, db: &mut toasty::Db) -> Result<Option<Vec<M>>, topcoat::Error> {
        if self.exhausted {
            return Ok(None);
        }
        let remaining = (MAX_EXPORT_ROWS + 1).saturating_sub(self.raw_scanned);
        if remaining == 0 {
            return Ok(None);
        }
        let take = remaining.min(EXPORT_CHUNK_ROWS);
        let mut page = toasty::stmt::Paginate::new(self.query.clone(), take);
        if let Some(cursor) = self.after.take() {
            page = page.after(cursor);
        }
        let loaded = page.exec(db).await.map_err(crate::db::unavailable)?;
        if loaded.items.is_empty() {
            self.exhausted = true;
            return Ok(None);
        }
        let full = loaded.items.len() == take;
        self.raw_scanned += loaded.items.len();
        self.after = loaded.next_cursor;
        let items = loaded.items;
        if !full {
            self.exhausted = true;
            self.after = None;
        }
        Ok(Some(items))
    }
    ```

    The `remaining == 0` branch is where the window ends without knowing whether
    rows remain.
  - Tests in the same file's `mod tests` (~line 704+): the `Dummy` model, the
    helper `seed_past_the_export_cap(db, name_fn)` (~line 727) that inserts exactly
    `MAX_EXPORT_ROWS + 1` rows with `create_many`, and
    `export_counts_only_viewable_rows_within_the_window` (~line 1601), which seeds
    exactly `MAX+1` rows alternating `allowed-`/`denied-` and expects a **200** with
    `(MAX+1).div_ceil(2)` rows. That test must stay green: nothing lies beyond its window.
- Known and separate: GH #232 (a full chunk carrying no cursor makes the walk
  rescan from the start). Do not fix it here.
- Docs asserting the contract: `docs/guide/src/tables.md:70` ("Capped at 10k
  viewable rows: per-row `can_view` runs before the cap, so 413 reflects what the
  caller may receive") and `docs/adr/0012-tenancy-grouping-export.md` (~line 27-29).

Conventions: comments cite GH issues and state *why*; tests for the export live in
this file's `mod tests` and drive the real router with `Panel::new("admin")…build()`
and `router.handle(request)` (copy `export_counts_only_viewable_rows_within_the_window`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Export tests | `cargo test -p argentum-core --lib export` | all pass |
| Whole workspace | `cargo test --workspace --locked` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |

## Scope

**In scope**: `crates/argentum-core/src/panel/actions.rs`,
`docs/guide/src/tables.md`, `docs/adr/0012-tenancy-grouping-export.md`.

**Out of scope**:
- GH #232 (cursor-less full chunk) — separate issue.
- Removing the two-pass design or raising `MAX_EXPORT_ROWS`.
- The CSV header on empty exports (a separate finding).

## Git workflow

- Branch: `advisor/003-export-no-silent-truncation`.
- Squash-merged as `fix(table): refuse an export whose scan window holds back viewable rows (#<issue>)`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Teach `ExportChunker` whether rows remain past the window

Add a field `beyond_window: bool` (initialized `false` in `ExportChunker::new`).
In `next_chunk`, replace the `remaining == 0` early return with:

```rust
if remaining == 0 {
    // The window is full (GH #<issue>): one probe row past it says whether the
    // walk stopped on the table's end or on the window's. A full chunk without
    // a cursor cannot be probed (GH #232) and is treated as the end.
    if let Some(cursor) = self.after.clone() {
        let probe = toasty::stmt::Paginate::new(self.query.clone(), 1)
            .after(cursor)
            .exec(db)
            .await
            .map_err(crate::db::unavailable)?;
        self.beyond_window = !probe.items.is_empty();
    }
    self.exhausted = true;
    return Ok(None);
}
```

Add `fn beyond_window(&self) -> bool { self.beyond_window }`. Update the struct's
rustdoc: it yields the raw window and reports whether rows remain past it.

**Verify**: `cargo check -p argentum-core --locked` → exit 0.

### Step 2: Refuse in phase 1, abort in phase 2

- Phase 1: after the `while` loop and **before** `enforce_export_cap_count(visible)?`, add:

  ```rust
  // The cap counts viewable rows, but only inside the raw window: rows left
  // past it may be viewable too, so a 200 would be a silent truncation.
  if chunker.beyond_window() {
      return Err(topcoat::router::error::content_too_large().into());
  }
  ```

- Phase 2: after its `loop` ends normally (the `Ok(None) => break` path), add a
  check: if the phase-2 chunker's `beyond_window()` is true, log with
  `tracing::error!(resource = R::slug(), "export window overflowed mid-stream")`
  and `tx.abort(std::io::Error::other("export overflowed its window"))`, then `return`.
  (Rows inserted between the passes can fill the window only in phase 2.)
- Update `resource_export`'s rustdoc paragraph (~line 372-381) and the
  `MAX_EXPORT_ROWS` comment (~line 279) to state the rule in present tense: a
  full raw window with rows beyond it is a 413, like a viewable count over the cap.

**Verify**: `cargo test -p argentum-core --lib export` → all existing export tests pass,
including `export_counts_only_viewable_rows_within_the_window` (still 200).

### Step 3: Regression test

Generalize the seed helper: add a parameter `rows: usize` to
`seed_past_the_export_cap` (rename it `seed_dummies` and update its two call sites
to pass `MAX_EXPORT_ROWS + 1`), or add a sibling helper — keep one batched
`create_many` insert either way.

Add `export_refuses_when_viewable_rows_lie_past_the_window`, copying
`export_counts_only_viewable_rows_within_the_window` but seeding
`MAX_EXPORT_ROWS + 1 + 20` rows (alternating allowed/denied, zero-padded names so
the default order interleaves them). Assert the response status is **413** and
that no body bytes are produced.

Test through the router only — do not add a configurable window to `ExportChunker`
just for tests.

**Verify**: `cargo test -p argentum-core --lib export` → all pass including the new test.

### Step 4: Docs

- `docs/guide/src/tables.md:70`: after "413 reflects what the caller may receive",
  add: "An export whose filtered set runs past the 10,001-row scan window is a
  413 too, even when fewer rows would be viewable: the export never returns a
  partial file."
- `docs/adr/0012-tenancy-grouping-export.md` Export paragraph: the same rule in
  one clause; add today's date to the header's `Amended:` list.

**Verify**: `cargo test --workspace --locked` → exit 0.

## Test plan

- New: `export_refuses_when_viewable_rows_lie_past_the_window` (413 past the window).
- Kept: `export_counts_only_viewable_rows_within_the_window` (200 exactly at the window),
  `export_chunker_stops_at_a_short_chunk`, the 413-on-cap tests.

## Done criteria

- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `grep -n "beyond_window" crates/argentum-core/src/panel/actions.rs` shows the field, the probe, and both phase checks
- [ ] `git status` shows changes only in the in-scope files
- [ ] `plans/README.md` status row updated

## STOP conditions

- `export_counts_only_viewable_rows_within_the_window` turns into a 413 after
  step 2 — the probe is finding a row that does not exist (cursor semantics
  differ from this plan's assumption). Report; do not loosen the test.
- `Paginate::…after(cursor)` with a limit of 1 errors on SQLite.
- The new test takes more than ~10 s locally (report the time; the seed must stay one batched insert).

## Maintenance notes

- If the export ever walks past the window (e.g. a larger raw bound for
  row-policy resources), `beyond_window` is still the signal for "stopped early".
- When GH #232 is fixed so full chunks always carry a cursor, remove the
  "cannot be probed" caveat in the comment.
- Reviewers: check the phase-1 check comes before any response bytes and that
  phase 2 aborts rather than finishing the stream.
