# Plan 004: Stop a date filter on the last representable day from panicking

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat cbb738a8..HEAD -- crates/argentum-core/src/resource/filter.rs examples/showcase/tests/filter_check.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `cbb738a8`, 2026-09-24

## Why this matters

`DateFilter::to_expr` turns a date-only value (`?filters=created_at:YYYY-MM-DD`)
into `>= midnight AND < midnight + 24h`. The addition uses jiff's
`Timestamp + Span`, which **panics** on overflow. `jiff::Timestamp::MAX` is
`9999-12-30T22:00:00.999999999Z`, so `9999-12-30` parses to a valid start and
the +24h overflows. Any user who can open a list with a `DateFilter` can send
that URL and get a 500 with a panic in the log. The same function runs on the
list page, the live-search shard, the export, and the "unapplied filters"
warning check, so every one of those paths is affected.

## Current state

- `crates/argentum-core/src/resource/filter.rs:180-205` — `DateFilter::to_expr`:

  ```rust
  pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
      let v = value.trim();
      if v.is_empty() {
          return None;
      }
      // Accept RFC3339 or YYYY-MM-DD (whole UTC day).
      if let Ok(ts) = v.parse::<jiff::Timestamp>() {
          return Some(self.lens.clone().eq(ts));
      }
      // … `+`-restoring retry (GH #93) …
      if let Ok(date) = v.parse::<jiff::civil::Date>() {
          let start: jiff::Timestamp = format!("{date}T00:00:00Z").parse().ok()?;
          let end = start + jiff::Span::new().hours(24);
          return Some(self.lens.clone().ge(start).and(self.lens.clone().lt(end)));
      }
      None
  }
  ```

  `9999-12-31` already returns `None` (its midnight is past `Timestamp::MAX`, so
  the `.ok()?` bails), and a `None` for a non-empty value is reported as an
  "invalid value" by `Table::unapplied_filters`
  (`crates/argentum-core/src/resource/table/mod.rs:388-412`) — the list warns and
  the export returns 400. That is the correct behavior for unparseable input.
- The only other jiff arithmetic in the crates is
  `examples/showcase/src/seed.rs:469`, which already uses `checked_add` — the
  pattern to follow.
- Tests: `filter.rs` `mod tests` (~line 400+) has
  `date_filter_date_only_matches_whole_day` (a `Task` model with `created_at`,
  in-memory SQLite, asserts the whole-day match) and
  `date_filter_recovers_plus_offsets_mangled_by_query_decode` (pure `to_expr`
  assertions). `examples/showcase/tests/filter_check.rs` hits
  `/admin/posts?filters=created_at:…` through the router (line ~113), using
  `full_db()`, `router(db)`, `demo_client(&router, &db)`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Filter unit tests | `cargo test -p argentum-core --lib date_filter` | all pass |
| Showcase filter tests | `cargo test -p showcase --test it filter_check::` | all pass |
| Whole workspace | `cargo test --workspace --locked` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo +nightly fmt --all -- --check` | exit 0 |

## Scope

**In scope**: `crates/argentum-core/src/resource/filter.rs`,
`examples/showcase/tests/filter_check.rs`.

**Out of scope**: the filter bar's date control rendering (`resource/table/render.rs`)
and `filters.js` — separate findings.

## Git workflow

- Branch: `advisor/004-date-filter-overflow-panic`.
- Squash-merged as `fix(table): bound a date-only filter at the last representable instant (#<issue>)`.
- Do NOT push or open a PR unless instructed.

## Steps

### Step 1: Replace the panicking addition

In `DateFilter::to_expr`, replace the `end` computation and return with:

```rust
let start: jiff::Timestamp = format!("{date}T00:00:00Z").parse().ok()?;
// The day's end can lie past `Timestamp::MAX` (9999-12-30): `+` would panic
// on a user-supplied URL, so the last day is bounded below only — no instant
// exists past the maximum for the upper bound to exclude.
return Some(match start.checked_add(jiff::Span::new().hours(24)) {
    Ok(end) => self.lens.clone().ge(start).and(self.lens.clone().lt(end)),
    Err(_) => self.lens.clone().ge(start),
});
```

Update the method's rustdoc (the paragraph above `to_expr`) only if it states
the upper bound as unconditional.

**Verify**: `cargo check -p argentum-core --locked` → exit 0.

### Step 2: Unit test

In `filter.rs` `mod tests`, extend `date_filter_recovers_plus_offsets_mangled_by_query_decode`
or add `date_filter_on_the_last_representable_day_does_not_panic`:

```rust
let f = DateFilter::r#for(Task::fields().created_at());
assert!(f.to_expr("9999-12-30").is_some(), "the last day builds a lower-bounded predicate");
assert!(f.to_expr("9999-12-31").is_none(), "a day past the maximum is invalid, not a panic");
assert!(f.to_expr("-009999-01-01").is_none(), "a day before the minimum is invalid");
```

(If the third line's input parses differently than stated — check by running it —
keep whichever of `is_some`/`is_none` jiff produces, as long as it does not panic,
and say so in a comment.)

**Verify**: `cargo test -p argentum-core --lib date_filter` → all pass including the new assertions.

### Step 3: Through the router

In `examples/showcase/tests/filter_check.rs`, add
`posts_date_filter_on_the_last_day_renders` modeled on the test around line 113:
`client.get("/admin/posts?filters=created_at:9999-12-30")` → assert
`resp.status().is_success()` and that the page renders no rows
(`row_titles(&body_string(resp).await).is_empty()`).
Also GET `/admin/posts/export?filters=created_at:9999-12-30` and assert status 200.

**Verify**: `cargo test -p showcase --test it filter_check::` → all pass.

## Test plan

- Unit: `to_expr` at and past the maximum day (step 2).
- Integration: list page and export with the boundary date return 200, not 500 (step 3).

## Done criteria

- [ ] `cargo test --workspace --locked` exits 0
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` exits 0
- [ ] `cargo +nightly fmt --all -- --check` exits 0
- [ ] `grep -n "start + jiff::Span" crates/argentum-core/src/resource/filter.rs` returns nothing
- [ ] `git status` shows changes only in the in-scope files
- [ ] `plans/README.md` status row updated

## STOP conditions

- `9999-12-30` does not panic on the unmodified code (run step 2's first assertion
  before step 1 to confirm the bug): report the jiff version from `Cargo.lock`.
- Any other `to_expr`/filter path in `filter.rs` does unchecked timestamp
  arithmetic you were not told about — list it rather than widening scope.

## Maintenance notes

- Any future date/range filter (e.g. "between two dates", "last N days") must
  use `checked_*` arithmetic on user input; consider a clippy
  `disallowed_methods`/`arithmetic_side_effects` rule scoped to `filter.rs` if
  more appear.
