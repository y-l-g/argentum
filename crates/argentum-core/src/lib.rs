//! Argentum — a server-rendered admin toolkit on Topcoat and Toasty.
//!
//! This crate holds the toolkit's foundation: the [`Resource`] trait and the
//! types that compose an admin UI (Panel, Table, Schema, Action). See the
//! workspace README and `CONTEXT.md` for the vocabulary and the roadmap.
#![doc = include_str!("../../../CONTEXT.md")]

// The full toolkit surface lands in later phases. For now, expose a single
// renderable component and the `Db` glue so the crate has vertical slices
// through Topcoat and Toasty.
pub mod db;
pub mod view;
