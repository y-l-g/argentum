//! One integration-test binary for the showcase (GH #179, ADR-0015).
//!
//! The per-file targets each linked the full stack — fourteen executables of
//! ~160-190 MB — so `cargo test --workspace` spent most of its link time
//! building the same dependencies over and over. Every file is a module here:
//! one binary, one link, and `tests/common` is compiled once.
//!
//! Filter per file with `cargo test --test it <module>::` (the module name is
//! the old file name). Accepted costs: a compile error in any module fails the
//! whole target, and there is no per-file binary isolation.

mod common;

mod admin;
mod auth_check;
mod blog_check;
mod bulk_check;
mod comments_check;
mod create_check;
mod delete_check;
mod detail_check;
mod detail_relation_check;
mod edit_check;
mod file_repeater_check;
mod filter_check;
mod group_export_check;
mod list_actions_check;
mod relation_check;
mod states_check;
mod tenancy_check;
mod upload_check;
