//! Argentum UI — beautiful Topcoat primitives owned as a library.
//!
//! This crate owns the styled `#[component]`s that `argentum-core` renders
//! (Table, Schema, Panel Shell). Apps never run `topcoat ui add`; they
//! depend on this crate and get the components as a library. See ADR-0006.

pub mod components;

// Re-export commonly used components at crate root for ergonomic `argentum_ui::card` etc.
pub use components::alert::{AlertVariant, alert, alert_description, alert_title};
pub use components::alert_dialog::alert_dialog;
pub use components::badge::{BadgeVariant, badge, badge_variants};
pub use components::button::{ButtonSize, ButtonVariant, button, button_variants};
pub use components::card::{
    card, card_content, card_description, card_footer, card_header, card_title,
};
pub use components::dialog::{
    dialog, dialog_content, dialog_description, dialog_footer, dialog_header, dialog_title,
};
pub use components::input::input;
pub use components::label::label;
pub use components::pagination::{
    pagination, pagination_content, pagination_ellipsis, pagination_item, pagination_link,
    pagination_next, pagination_previous,
};
pub use components::separator::{SeparatorOrientation, separator};
pub use components::sheet::{SheetSide, sheet, sheet_content};
pub use components::sidebar::{
    sidebar, sidebar_content, sidebar_footer, sidebar_group, sidebar_group_content,
    sidebar_group_label, sidebar_header, sidebar_menu, sidebar_menu_button, sidebar_menu_item,
    sidebar_separator, sidebar_trigger,
};
pub use components::skeleton::skeleton;
pub use components::table::{
    table, table_body, table_caption, table_cell, table_footer, table_head, table_header, table_row,
};
// Composites — owned Argentum components (ADR-0007). Re-exported here for
// ergonomic `argentum_ui::page` etc.; they live in `components/composites/`.
pub use components::composites::code_block::code_block;
pub use components::composites::page::{
    page, page_content, page_description, page_header, page_title,
};

/// Tailwind build helper for the per-app contract.
///
/// Call from the app's `build.rs`:
///
/// ```ignore
/// fn main() {
///     argentum_ui::tailwind_build().unwrap();
/// }
/// ```
///
/// This scans `styles.css` (which must `@import "tailwindcss"` and contain
/// `@source` globs for the app's `src/` and for `argentum-ui/src/`) and
/// writes `$OUT_DIR/tailwind.css` for `tailwind::stylesheet!()`.
pub fn tailwind_build() -> Result<std::path::PathBuf, topcoat::tailwind::BuildError> {
    topcoat::tailwind::BuildConfig::new()
        .input("styles.css")
        .render()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tailwind_build_fn_exists() {
        // Just proves the symbol is linkable; actual build needs styles.css
        let _ = tailwind_build as fn() -> Result<std::path::PathBuf, topcoat::tailwind::BuildError>;
    }
}
