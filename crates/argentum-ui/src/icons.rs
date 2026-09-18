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
/// Toast success.
pub const CIRCLE_CHECK: IconData = iconify_icon!("lucide:circle-check");
/// Toast info.
pub const INFO: IconData = iconify_icon!("lucide:info");
/// Toast warning.
pub const TRIANGLE_ALERT: IconData = iconify_icon!("lucide:triangle-alert");
/// Toast error.
pub const OCTAGON_X: IconData = iconify_icon!("lucide:octagon-x");
/// Toast close button.
pub const X: IconData = iconify_icon!("lucide:x");
