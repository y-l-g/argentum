//! One integration-test binary for `argentum-core` (GH #179, ADR-0015).
//!
//! Same consolidation as the showcase tests: three separate targets each linked
//! the full topcoat/toasty stack (~80 MB apiece) to run a handful of tests. The
//! per-file targets are modules here, so one link covers all of them.
//!
//! Filter per file with `cargo test -p argentum-core --test it <module>::`.

mod auth_override;
mod resource_query_override;
mod sqlite;

mod embedded_lens;
mod readonly_render;
