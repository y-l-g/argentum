//! Tablo UI — beautiful Topcoat primitives owned as a library.
//!
//! This crate owns the styled `#[component]`s that `tablo-core` renders
//! (Table, Schema, Panel Shell). Apps never run `topcoat ui add`; they
//! depend on this crate and get the components as a library. See ADR-0006.

pub mod components;
pub mod icons;

// Re-exported at the crate root for ergonomic `tablo_ui::card` etc. The
// `primitives` half mirrors the vendored `topcoat-ui-registry` components
// verbatim (sync with `cargo xtask sync-topcoat-ui`, ADR-0007 — edit the
// registry, not these); the `composites` half is owned Tablo code in
// `components/composites/`.
pub use components::{
    composites::{
        error_state::error_state,
        page::{page, page_content, page_description, page_header, page_title},
        theme::theme_init_script,
        toast::{
            toast, toast_close, toast_content, toast_description, toast_icon, toast_title, toaster,
        },
    },
    primitives::{
        alert::{AlertVariant, alert, alert_description, alert_title},
        alert_dialog::alert_dialog,
        button::{ButtonSize, ButtonVariant, button, button_variants},
        card::{card, card_content, card_description, card_footer, card_header, card_title},
        checkbox::checkbox,
        dialog::{
            dialog, dialog_content, dialog_description, dialog_footer, dialog_header, dialog_title,
        },
        field::{
            FieldLegendVariant, FieldOrientation, field, field_content, field_description,
            field_error, field_group, field_label, field_legend, field_separator, field_set,
            field_title,
        },
        input::input,
        label::label,
        pagination::{
            pagination, pagination_content, pagination_ellipsis, pagination_item, pagination_link,
            pagination_next, pagination_previous,
        },
        select::select,
        separator::{SeparatorOrientation, separator},
        sheet::{SheetSide, sheet, sheet_content},
        sidebar::{
            SidebarCollapsible, SidebarMenuButtonSize, SidebarMenuButtonVariant, SidebarSide,
            SidebarVariant, sidebar, sidebar_content, sidebar_footer, sidebar_group,
            sidebar_group_action, sidebar_group_content, sidebar_group_label, sidebar_header,
            sidebar_input, sidebar_inset, sidebar_menu, sidebar_menu_action, sidebar_menu_badge,
            sidebar_menu_button, sidebar_menu_button_variants, sidebar_menu_item,
            sidebar_menu_skeleton, sidebar_menu_sub, sidebar_menu_sub_button,
            sidebar_menu_sub_item, sidebar_provider, sidebar_rail, sidebar_separator,
            sidebar_trigger,
        },
        skeleton::skeleton,
        table::{
            table, table_body, table_caption, table_cell, table_footer, table_head, table_header,
            table_row,
        },
        textarea::textarea,
    },
};

// Assets for shell JS — via `asset!` + `AssetBundle` + `topcoat::runtime::script()` (ADR-0009 /
// T28.5)
pub const SIDEBAR_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/sidebar.js");
pub const THEME_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/theme.js");
pub const DIALOG_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/dialog.js");
pub const BULK_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/bulk.js");
pub const FILTERS_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/filters.js");
pub const LIVE_SEARCH_JS: topcoat::asset::Asset =
    topcoat::asset::asset!("../assets/live-search.js");
pub const SELECTS_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/selects.js");
pub const VARIANT_JS: topcoat::asset::Asset = topcoat::asset::asset!("../assets/variant.js");
pub const NOTIFICATION_JS: topcoat::asset::Asset =
    topcoat::asset::asset!("../assets/notifications.js");
pub const MUTATION_SUBMIT_JS: topcoat::asset::Asset =
    topcoat::asset::asset!("../assets/mutation-submit.js");

/// Tailwind build helper for the per-app contract.
///
/// Call from the app's `build.rs`:
///
/// ```ignore
/// fn main() {
///     tablo_ui::tailwind_build().unwrap();
/// }
/// ```
///
/// This scans `styles.css` (which must `@import "tailwindcss"` and contain
/// `@source` globs for the app's `src/` and for `tablo-ui/src/`) and
/// writes `$OUT_DIR/tailwind.css` for `tailwind::stylesheet!()`.
pub fn tailwind_build() -> Result<std::path::PathBuf, topcoat::tailwind::BuildError> {
    topcoat::tailwind::BuildConfig::new()
        .input("styles.css")
        .render()
}
