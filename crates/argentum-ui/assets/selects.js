// Option search for Argentum selects (GH #91, GH #150, GH #184).
//
// A `Select::searchable()` renders an `input[data-options-filter]` and an empty
// `ul[data-options-list]` above its `<select>`, inside
// `[data-select-filterable]`.
//
// The list is the point. Narrowing the native `<select>`'s own options is not
// something a page can show: the primitive opts into `appearance: base-select`,
// whose popup is browser chrome that ignores `option[hidden]` — so the filter
// used to look inert, and the only way to see its effect was to open the
// select and read its options (GH #184). This script therefore renders its own
// filtered listbox from the select's options, and writes the chosen value back
// onto the select, which stays the form control and the no-JS fallback.
//
// * Bounded sets: typing narrows the list by label substring
//   (case-insensitive); the placeholder option always stays.
// * Overflowed relationship sets (GH #150): the wrapper carries
//   `data-options-server="true"` + `data-options-field="<name>"` (+
//   `data-options-overflow` on initial render). Typing debounces (200ms,
//   abort in-flight) a `GET {parent_list_url}/options?field=&q=` fetch that
//   replaces the `<select>` options with server markup, preserving the
//   current selection and the placeholder; the list re-renders from the
//   replaced options. The hint `[data-options-hint]` ("Too many options —
//   type to search") stays until the server narrows.
//   Without JS the input is inert and the plain select keeps working (stored
//   value kept, relation cannot be changed past the cap).
//
// The server renders both controls so the field works without this script;
// with it, the native `<select>` is hidden once the combobox over it is wired
// (GH #236). Hiding is display only: the select stays in the markup as the
// submitted value carrier, and `partsOf` still resolves it as a descendant of
// `[data-select-filterable]`.
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

// --- the visible listbox -----------------------------------------------------

// How many rows the listbox renders at once. It is an affordance, not the
// source of truth: the server owns the real bound (GH #150), so this only
// keeps the DOM small.
const MAX_LIST_ITEMS = 50;

// Which options a needle offers, as plain data — the one pure decision this
// script makes, split out so it can be unit-tested without a DOM.
//
// An option with an empty `value` is the placeholder ("-- Select --"): it is
// offered whenever the needle is empty (the way back to "no choice") and is
// never counted against the cap, so the cap always buys `limit` real choices.
function matchingOptions(options, needle, limit) {
  const lowered = needle.trim().toLowerCase();
  const rows = [];
  let choices = 0;
  for (const opt of options) {
    if (opt.value === '') {
      if (lowered === '') rows.push({ ...opt });
      continue;
    }
    if (lowered !== '' && !opt.label.toLowerCase().includes(lowered)) continue;
    if (choices >= limit) break;
    choices += 1;
    rows.push({ ...opt });
  }
  return rows;
}

// The parts of the combobox, resolved from either the filter or the list.
function partsOf(node) {
  const combo = node.closest('[data-options-combobox]');
  const wrap = node.closest('[data-select-filterable]');
  return {
    combo,
    wrap,
    filter: combo && combo.querySelector('[data-options-filter]'),
    list: combo && combo.querySelector('[data-options-list]'),
    select: wrap && wrap.querySelector('select'),
  };
}

// The `<option>`s worth offering for `needle`, as plain data.
//
// `value` comes from the option's property rather than its attribute: an
// option with no `value` falls back to its text, which is exactly what the
// browser would submit.
function optionRows(select, needle) {
  const options = Array.from(select.querySelectorAll('option')).map((opt) => ({
    // An option with no `value` submits its text, so that is its value here
    // too; only a genuinely empty value is the placeholder.
    value: opt.value,
    label: (opt.textContent || '').trim(),
    selected: opt.selected,
  }));
  return matchingOptions(options, needle, MAX_LIST_ITEMS);
}

function closeList(combo) {
  const list = combo.querySelector('[data-options-list]');
  if (!list) return;
  list.hidden = true;
  list.replaceChildren();
}

function messageRow(list, text) {
  const item = document.createElement('li');
  item.className = 'px-2 py-1.5 text-muted-foreground';
  item.textContent = text;
  list.replaceChildren(item);
  list.hidden = false;
}

function renderList({ combo, filter, list, select }) {
  if (!combo || !filter || !list || !select) return;
  const needle = filter.value.trim();
  const rows = optionRows(select, needle);
  if (rows.length === 0) {
    // "Nothing matched" is the whole reason this list exists, so it says so
    // instead of leaving an empty box.
    messageRow(list, needle === '' ? 'No options' : 'No matching options');
    return;
  }
  list.replaceChildren();
  rows.forEach((row) => {
    const item = document.createElement('li');
    item.setAttribute('role', 'option');
    item.setAttribute('aria-selected', row.selected ? 'true' : 'false');
    item.dataset.value = row.value;
    item.className =
      'cursor-pointer rounded-md px-2 py-1.5 hover:bg-foreground/5'
      + (row.selected ? ' font-medium' : '');
    item.textContent = row.label;
    list.appendChild(item);
  });
  list.hidden = false;
}

// The option behind a list row, matched on the property (never an escaped
// selector: a PK is opaque and may contain anything).
function optionFor(select, value) {
  return Array.from(select.options).find((opt) => opt.value === value) || null;
}

// Commit a choice: write it onto the select — the form control — and announce
// it, so a value-bound field stays in step.
function chooseOption({ combo, filter, select }, option) {
  if (!option) return;
  select.value = option.value;
  if (filter) filter.value = (option.textContent || '').trim();
  select.dispatchEvent(new Event('change', { bubbles: true }));
  closeList(combo);
}

// The row the keyboard is on, or the first one.
function activeItem(list) {
  return (
    list.querySelector('[role="option"][aria-selected="true"]')
    || list.querySelector('[role="option"]')
  );
}

// --- the control the combobox replaces ---------------------------------------

// Whether the script hides the native `<select>` behind its combobox.
//
// All four parts must resolve: a field whose markup is incomplete keeps a
// visible control rather than losing the only one it has. Hiding is display
// only — the select stays in the markup as the submitted value carrier, and
// `partsOf` still resolves it as a descendant of `[data-select-filterable]`.
//
// A hidden control is barred from constraint validation, so a `required`
// select no longer fails the browser's own check; the server's required check
// is what enforces it.
function shouldHideNativeSelect({ combo, filter, list, select }) {
  return Boolean(combo && filter && list && select);
}

// The element carrying the replaced control: the native `<select>`'s own
// wrapper when the select primitive renders one — the chevron lives there, and
// hiding the `<select>` alone would leave it behind — and the `<select>`
// itself otherwise. Never the field wrapper: the combobox, the overflow hint,
// and the error message live inside it.
function nativeControl({ wrap, select }) {
  const parent = select.parentElement;
  return parent && parent !== wrap ? parent : select;
}

function hideNativeSelect(parts) {
  if (!shouldHideNativeSelect(parts)) return;
  nativeControl(parts).hidden = true;
}

// Hide the control behind every wired combobox in `root`, or in the document.
// A swap replaces the field markup wholesale, so the pass runs again for the
// nodes that arrive later (see `install`).
function applyHiddenSelects(root) {
  (root || document).querySelectorAll('[data-options-filter]').forEach((filter) => {
    hideNativeSelect(partsOf(filter));
  });
}

// --- wiring ------------------------------------------------------------------

// Everything below only makes sense with a document. It lives in a function so
// this file can also be `require`d by its Node unit test (see the export at the
// bottom), which has no DOM: loading the script must not touch one.
function install() {
  document.addEventListener('input', (e) => {
    if (!e.target.closest('[data-options-filter]')) return;
    const parts = partsOf(e.target);
    if (!parts.select) return;
    const needle = parts.filter.value.trim();
    const field = parts.wrap.getAttribute('data-options-field');
    const server = parts.wrap.getAttribute('data-options-server') === 'true';
    if (server && field) {
      messageRow(parts.list, 'Searching…');
      parts.wrap.dataset.optionsSearching = 'true';
      const prevTimer = serverTimers.get(parts.filter);
      if (prevTimer) clearTimeout(prevTimer);
      const timer = setTimeout(async () => {
        serverTimers.delete(parts.filter);
        await serverSearch(parts.filter, parts.wrap, parts.select, field, needle);
        delete parts.wrap.dataset.optionsSearching;
        renderList(partsOf(parts.filter));
      }, 200);
      serverTimers.set(parts.filter, timer);
      return;
    }
    renderList(parts);
  });

  // Entering the field opens the list, so the filter shows what it is filtering.
  document.addEventListener('focusin', (e) => {
    if (!e.target.closest('[data-options-filter]')) return;
    renderList(partsOf(e.target));
  });

  // Picking a row: `mousedown` + preventDefault so the input keeps focus and the
  // click is not lost to a blur before it lands.
  document.addEventListener('mousedown', (e) => {
    const item = e.target.closest('[data-options-list] [role="option"]');
    if (!item) return;
    e.preventDefault();
    const parts = partsOf(item);
    if (!parts.select) return;
    chooseOption(parts, optionFor(parts.select, item.dataset.value));
  });

  // Arrows move through the list, Enter picks, Escape closes. Focus stays in the
  // input, which is what makes typing-to-narrow continuous.
  document.addEventListener('keydown', (e) => {
    if (!e.target.closest('[data-options-filter]')) return;
    const parts = partsOf(e.target);
    if (!parts.select || !parts.list) return;
    if (parts.list.hidden) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        renderList(parts);
      }
      return;
    }
    const items = Array.from(parts.list.querySelectorAll('[role="option"]'));
    if (items.length === 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const current = items.indexOf(activeItem(parts.list));
      const step = e.key === 'ArrowDown' ? 1 : -1;
      const next = items[(current + step + items.length) % items.length];
      items.forEach((item) => item.setAttribute('aria-selected', 'false'));
      next.setAttribute('aria-selected', 'true');
      next.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const item = activeItem(parts.list);
      chooseOption(parts, item && optionFor(parts.select, item.dataset.value));
    } else if (e.key === 'Escape') {
      closeList(parts.combo);
    }
  });

  // Tab away: close. Captured, because `focusout` does not bubble usefully here.
  document.addEventListener(
    'focusout',
    (e) => {
      const combo = e.target.closest && e.target.closest('[data-options-combobox]');
      if (!combo || combo.contains(e.relatedTarget)) return;
      closeList(combo);
    },
    true,
  );

  // A click outside the field closes its list, so it never outlives the field.
  document.addEventListener('click', (e) => {
    document.querySelectorAll('[data-options-combobox]').forEach((combo) => {
      if (!combo.contains(e.target)) closeList(combo);
    });
  });

  // A server fetch replaces the whole option set *after* the list was rendered,
  // so re-render when it lands — but only while the user is still in the field,
  // or an unrelated `change` would pop the list open.
  document.addEventListener(
    'change',
    (e) => {
      const select = e.target.closest && e.target.closest('[data-select-filterable] select');
      if (!select) return;
      const wrap = select.closest('[data-select-filterable]');
      if (!wrap || wrap.dataset.optionsSearching === 'true') return;
      const parts = partsOf(select);
      if (parts.filter && document.activeElement === parts.filter) {
        renderList(parts);
      }
    },
    true,
  );

  // The hide pass runs on install and again for markup that arrives later: a
  // swap replaces the field without a page load, and no `load` or
  // `DOMContentLoaded` fires for it.
  applyHiddenSelects();
  if (typeof MutationObserver !== 'undefined' && document.documentElement) {
    new MutationObserver((records) => {
      records.forEach((record) => {
        record.addedNodes.forEach((node) => {
          if (node.nodeType === 1) applyHiddenSelects(node);
        });
      });
    }).observe(document.documentElement, { childList: true, subtree: true });
  }
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`selects.test.js`). There is no JS test
// runner in this workspace and this file must stay a plain browser script
// loaded through `asset!`, so it cannot be an ES module. The guard keeps the
// browser branch inert.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { matchingOptions, MAX_LIST_ITEMS, shouldHideNativeSelect };
}
