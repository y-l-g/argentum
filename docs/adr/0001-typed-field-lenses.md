# Typed field lenses, not string state paths

Date: 2026-08-19 — Status: accepted — Supersedes: none

Every Schema field and Table column binds through a typed Toasty field lens (`User::fields().email()`), never a string `statePath`. The lens carries nullability, uniqueness, column renames, and type, so hydration (Model → Schema) and dehydration (Schema → Create/Update) are compile-time checked. Filament's `"data.author.name"` magic, and its `data_set`/`data_get` runtime, has no place in Rust.

## Amendment (2026-09-10)

`required` now defaults from lens nullability (GH #100), but uniqueness metadata, storage names, and instance→field extraction remain upstream gaps — form values stay string-keyed (`HashMap<String, String>`) at the value level, so the lens proves field existence, not typed data flow. See `EXTERNAL_GAPS.md`.
