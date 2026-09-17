// Bulk selection for Argentum tables (GH #74, GH #151).
//
// Tables render one checkbox per row (`input[data-row-select]`, value = row
// key) plus a header select-all (`input[data-bulk-select-all]`). The bulk
// form keeps its single hidden `input[name=ids]` field as the transport: on
// submit this listener joins the checked keys into it (comma-separated, the
// format `resource_bulk_delete` already parses), so the server contract is
// unchanged.
//
// There is no visible ids input (GH #151): the destructive submit ships
// disabled and only this script enables it while at least one row is
// checked, so an empty bulk submit cannot be produced from the UI.
//
// Document-level delegation (like sidebar.js) so streamed/shard swaps that
// replace table markup need no re-installation. Scoped per table via
// `[data-table-root]` so multiple tables never cross-talk.
function updateBulkSubmit(root) {
  const submit = root.querySelector('[data-bulk-submit]');
  const boxes = Array.from(root.querySelectorAll('input[data-row-select]'));
  const checked = boxes.filter((cb) => cb.checked);
  if (submit) submit.disabled = checked.length === 0;
  // Tri-state header (GH #160): checked only when every row is checked,
  // indeterminate on a partial selection — otherwise a select-all followed
  // by one uncheck leaves the header lying checked.
  const all = root.querySelector('input[data-bulk-select-all]');
  if (all) {
    all.checked = boxes.length > 0 && checked.length === boxes.length;
    all.indeterminate = checked.length > 0 && checked.length < boxes.length;
  }
}

document.addEventListener('change', (e) => {
  const all = e.target.closest('[data-bulk-select-all]');
  if (all) {
    const root = all.closest('[data-table-root]') || document;
    const checked = all.checked;
    root.querySelectorAll('input[data-row-select]').forEach((cb) => {
      cb.checked = checked;
    });
    updateBulkSubmit(root);
    return;
  }
  const row = e.target.closest('input[data-row-select]');
  if (row) {
    const root = row.closest('[data-table-root]') || document;
    updateBulkSubmit(root);
  }
});

// A reload restores checkbox state before DOMContentLoaded; re-sync the
// server-rendered disabled state with what is actually checked.
document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-table-root]').forEach((root) => {
    updateBulkSubmit(root);
  });
});

document.addEventListener('submit', (e) => {
  const form = e.target.closest('form[data-bulk-form]');
  if (!form) return;
  const root = form.closest('[data-table-root]') || document;
  const checked = Array.from(root.querySelectorAll('input[data-row-select]:checked')).map(
    (cb) => cb.value,
  );
  const ids = form.querySelector('input[name="ids"]');
  if (ids) ids.value = checked.join(',');
});
