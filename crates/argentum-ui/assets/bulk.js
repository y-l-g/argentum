// Bulk selection for Argentum tables (GH #74).
//
// Tables render one checkbox per row (`input[data-row-select]`, value = row
// key) plus a header select-all (`input[data-bulk-select-all]`). The bulk
// form keeps its single `input[name=ids]` text field as the transport and the
// no-JS fallback: on submit this listener joins the checked keys into it
// (comma-separated, the format `resource_bulk_delete` already parses), so the
// server contract is unchanged. Without JS the text field still works by hand.
//
// Document-level delegation (like sidebar.js) so streamed/shard swaps that
// replace table markup need no re-installation. Scoped per table via
// `[data-table-root]` so multiple tables never cross-talk.
document.addEventListener('change', (e) => {
  const all = e.target.closest('[data-bulk-select-all]');
  if (all) {
    const root = all.closest('[data-table-root]') || document;
    const checked = all.checked;
    root.querySelectorAll('input[data-row-select]').forEach((cb) => {
      cb.checked = checked;
    });
  }
});

document.addEventListener('submit', (e) => {
  const form = e.target.closest('form[data-bulk-form]');
  if (!form) return;
  const root = form.closest('[data-table-root]') || document;
  const checked = Array.from(root.querySelectorAll('input[data-row-select]:checked')).map(
    (cb) => cb.value,
  );
  if (checked.length > 0) {
    const ids = form.querySelector('input[name="ids"]');
    if (ids) ids.value = checked.join(',');
  }
});
