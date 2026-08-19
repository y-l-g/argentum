//! The `user-list` example: a Topcoat `#[page]` listing Toasty User rows from
//! in-memory sqlite.
//!
//! The page, model, loader, and seed live in this library so integration tests
//! can drive the full request → render → response path. The binary only wires
//! the database, seeds it, and serves.

pub mod app;
pub mod models;
