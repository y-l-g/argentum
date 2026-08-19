# Typed field lenses, not string state paths

Every Schema field and Table column binds through a typed Toasty field lens (`User::fields().email()`), never a string `statePath`. The lens carries nullability, uniqueness, column renames, and type, so hydration (Model → Schema) and dehydration (Schema → Create/Update) are compile-time checked. Filament's `"data.author.name"` magic, and its `data_set`/`data_get` runtime, has no place in Rust.
