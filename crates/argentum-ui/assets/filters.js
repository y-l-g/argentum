// Typed filter controls for Argentum tables (GH #74, GH #151).
//
// The filter form keeps one hidden `input[name=filters]` transport
// (`key:value,key2:value2`, parsed by `TableState`). Typed controls carry only
// `data-filter-name` (no `name`, so they never submit on their own): on change
// this listener composes every control into the transport and submits the
// form, so filters apply immediately without an Apply button. The transport is
// rewritten even when no control has a value, so selecting "All" clears the
// filter instead of resubmitting the stale server-rendered value (GH #151).
// Without JS the free-text input + Apply button inside `<noscript>` keep the
// old path.
//
// Document-level delegation (like sidebar.js) so streamed/shard swaps that
// replace table markup need no re-installation.
function composeFilters(form) {
  const parts = [];
  form.querySelectorAll('[data-filter-name]').forEach((el) => {
    const name = el.getAttribute('data-filter-name');
    const value = (el.value || '').trim();
    if (name && value && value !== 'all') {
      parts.push(name + ':' + value);
    }
  });
  const transport = form.querySelector('input[data-filters-transport]');
  if (transport) transport.value = parts.join(',');
}

document.addEventListener('change', (e) => {
  const control = e.target.closest('[data-filter-name]');
  if (!control) return;
  const form = control.closest('form[data-filters-form]');
  if (!form) return;
  composeFilters(form);
  if (typeof form.requestSubmit === 'function') {
    form.requestSubmit();
  } else {
    form.submit();
  }
});

// Implicit submits (e.g. Enter in a date field) compose too, so a stale
// transport value can never ride along.
document.addEventListener('submit', (e) => {
  const form = e.target.closest('form[data-filters-form]');
  if (!form) return;
  composeFilters(form);
});
