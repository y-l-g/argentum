// Typed filter controls for Argentum tables (GH #74, GH #151).
//
// The filter form keeps one hidden `input[name=filters]` transport
// (`key:value,key2:value2`, parsed by `TableState`). Typed controls carry only
// `data-filter-name` (no `name`, so they never submit on their own): on change
// the control values are composed into the transport.
//
// A form marked `data-filters-live` belongs to a live table: the transport is
// bound to the runtime's `filters` signal, so this script composes the value
// and dispatches a bubbling `change` into the transport, which the runtime
// turns into a signal write — the shard re-renders the table in place, no
// navigation and no scroll jump. The rewritten transport is unconditional, so
// selecting "All" clears the filter instead of resubmitting the stale value.
// Without a live marker the composed transport is submitted as a GET form (a
// full navigation, the no-JS behaviour), and the `<noscript>` free-text
// fallback stays for scriptless readers.
//
// Document-level delegation (like bulk.js) so streamed/shard swaps that
// replace table markup need no re-installation.
function composeFilters(form) {
  const parts = [];
  form.querySelectorAll('[data-filter-name]').forEach((el) => {
    const name = el.getAttribute('data-filter-name');
    const value = (el.value || '').trim();
    // The All option is `value=""`, so the empty skip is the whole rule: a
    // genuine filter value of `"all"` must round-trip (GH #160).
    if (name && value) {
      parts.push(name + ':' + value);
    }
  });
  return parts.join(',');
}

document.addEventListener('change', (e) => {
  const control = e.target.closest('[data-filter-name]');
  if (!control) return;
  const form = control.closest('form[data-filters-form]');
  if (!form) return;
  const transport = form.querySelector('input[data-filters-transport]');
  if (transport) transport.value = composeFilters(form);
  if (form.hasAttribute('data-filters-live')) {
    if (transport) transport.dispatchEvent(new Event('change', { bubbles: true }));
    return;
  }
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
  const transport = form.querySelector('input[data-filters-transport]');
  if (transport) transport.value = composeFilters(form);
});
