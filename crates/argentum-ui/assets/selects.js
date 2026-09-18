// Client-side option search for Argentum selects (GH #91, GH #150).
//
// A `Select::searchable()` renders an `input[data-options-filter]` above its
// `<select>` inside `[data-select-filterable]`.
//
// * Bounded sets: typing narrows the rendered options by label substring
//   (case-insensitive); the placeholder option always stays.
// * Overflowed relationship sets (GH #150): the wrapper carries
//   `data-options-server="true"` + `data-options-field="<name>"` (+ 
//   `data-options-overflow` on initial render). Typing debounces (200ms,
//   abort in-flight) a `GET {parent_list_url}/options?field=&q=` fetch that
//   replaces the `<select>` options with server markup, preserving the
//   current selection and the placeholder. The hint `[data-options-hint]`
//   ("Too many options — type to search") stays until the server narrows.
//   Without JS the input is inert and the plain select keeps working (stored
//   value kept, relation cannot be changed past the cap).
//
// Document-level delegation (like bulk.js) so streamed/shard swaps that
// replace form markup need no re-installation.

const serverTimers = new WeakMap();
const serverControllers = new WeakMap();

function parentOptionsBase() {
  const path = window.location.pathname.replace(/\/$/, '');
  // /admin/posts/create -> /admin/posts ; /admin/posts/<id>/edit -> /admin/posts
  const base = path
    .replace(/\/create$/, '')
    .replace(/\/[^\/]+\/edit$/, '');
  return `${base}/options`;
}

function escapeAttr(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

async function serverSearch(filter, wrap, select, field, needle) {
  const prev = serverControllers.get(filter);
  if (prev) prev.abort();
  const controller = new AbortController();
  serverControllers.set(filter, controller);
  const current = select.value;
  const url = `${parentOptionsBase()}?field=${encodeURIComponent(field)}&q=${encodeURIComponent(needle)}`;
  let html;
  try {
    const res = await fetch(url, {
      headers: { Accept: 'text/html' },
      signal: controller.signal,
    });
    if (!res.ok) return;
    html = await res.text();
  } catch (err) {
    if (err && err.name === 'AbortError') return;
    return;
  } finally {
    if (serverControllers.get(filter) === controller) {
      serverControllers.delete(filter);
    }
  }
  const placeholder = select.querySelector('option[value=""]');
  const placeholderHtml = placeholder ? placeholder.outerHTML : '<option value="">-- Select --</option>';
  // Preserve the current selection across swaps (D2): the server never
  // echoes it, so re-attach when absent. Escape the PK for attribute use.
  const escCurrent = escapeAttr(current);
  const hasCurrent = current !== '' && html.includes(`value="${escCurrent}"`);
  const preserved = current !== '' && !hasCurrent
    ? `<option value="${escCurrent}" selected>${escCurrent}</option>`
    : '';
  // Mark the fetched current as selected when it matches (string replace,
  // no regex: PKs are opaque strings).
  if (current !== '' && hasCurrent) {
    html = html.split(`value="${escCurrent}"`).join(`value="${escCurrent}" selected`);
  }
  select.innerHTML = `${placeholderHtml}${preserved}${html}`;
}

document.addEventListener('input', (e) => {
  const filter = e.target.closest('[data-options-filter]');
  if (!filter) return;
  const wrap = filter.closest('[data-select-filterable]');
  if (!wrap) return;
  const select = wrap.querySelector('select');
  if (!select) return;
  const needle = filter.value.trim();
  const field = wrap.getAttribute('data-options-field');
  const server = wrap.getAttribute('data-options-server') === 'true';
  if (server && field) {
    const prevTimer = serverTimers.get(filter);
    if (prevTimer) clearTimeout(prevTimer);
    const timer = setTimeout(() => {
      serverTimers.delete(filter);
      serverSearch(filter, wrap, select, field, needle);
    }, 200);
    serverTimers.set(filter, timer);
    return;
  }
  const lowered = needle.toLowerCase();
  wrap.querySelectorAll('select option').forEach((opt) => {
    if (opt.value === '') return;
    const label = (opt.textContent || '').toLowerCase();
    opt.hidden = lowered !== '' && !label.includes(lowered);
  });
});
