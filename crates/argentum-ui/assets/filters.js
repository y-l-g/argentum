// Typed filter controls for Argentum tables (GH #74).
//
// The filter form keeps its single `input[name=filters]` text field
// (`key:value,key2:value2`, parsed by `TableState`) as the transport and the
// no-JS fallback. Typed controls carry only `data-filter-name` (no `name`, so
// they never submit on their own): on submit this listener composes them into
// the text field when at least one has a value, otherwise the hand-typed
// free-text value is left untouched. The server contract is unchanged.
//
// Document-level delegation (like sidebar.js) so streamed/shard swaps that
// replace table markup need no re-installation.
document.addEventListener('submit', (e) => {
  const form = e.target.closest('form[data-filters-form]');
  if (!form) return;
  const parts = [];
  form.querySelectorAll('[data-filter-name]').forEach((el) => {
    const name = el.getAttribute('data-filter-name');
    const value = (el.value || '').trim();
    if (name && value && value !== 'all') {
      parts.push(name + ':' + value);
    }
  });
  if (parts.length > 0) {
    const hidden = form.querySelector('input[name="filters"]');
    if (hidden) hidden.value = parts.join(',');
  }
});
