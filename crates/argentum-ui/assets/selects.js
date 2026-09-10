// Client-side option search for Argentum selects (GH #91).
//
// A `Select::searchable()` renders an `input[data-options-filter]` above its
// `<select>` inside `[data-select-filterable]`. Typing narrows the rendered
// options by label substring (case-insensitive); the placeholder option
// always stays. This covers the bounded option set only — relationship
// loads are capped server-side, and over-cap tables fail visibly instead.
// Without JS the input is inert and the plain select keeps working.
//
// Document-level delegation (like bulk.js) so streamed/shard swaps that
// replace form markup need no re-installation.
document.addEventListener('input', (e) => {
  const filter = e.target.closest('[data-options-filter]');
  if (!filter) return;
  const wrap = filter.closest('[data-select-filterable]') || document;
  const needle = filter.value.trim().toLowerCase();
  wrap.querySelectorAll('select option').forEach((opt) => {
    if (opt.value === '') return;
    const label = (opt.textContent || '').toLowerCase();
    opt.hidden = needle !== '' && !label.includes(needle);
  });
});
