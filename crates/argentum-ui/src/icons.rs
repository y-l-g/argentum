//! The app's Lucide icons, resolved once from the staged icon set.
//!
//! `iconify_icon!` resolves against the **compiling crate's** staged sets
//! (ADR-0007 stages Lucide in this crate's build script), so the consts live
//! here and `argentum-core` renders them with `icon` instead of staging a
//! second copy of the set.

use topcoat::icon::{IconData, iconify::iconify_icon};

/// Sortable column, neither direction active.
pub const ARROW_UP_DOWN: IconData = iconify_icon!("lucide:arrow-up-down");
/// Sortable column, sorted ascending.
pub const ARROW_UP: IconData = iconify_icon!("lucide:arrow-up");
/// Sortable column, sorted descending.
pub const ARROW_DOWN: IconData = iconify_icon!("lucide:arrow-down");
/// Searchable column: the prefix search covers it.
pub const SEARCH: IconData = iconify_icon!("lucide:search");
