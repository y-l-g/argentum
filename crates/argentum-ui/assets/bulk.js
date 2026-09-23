// Bulk selection for Argentum tables (GH #74, GH #151, GH #166, GH #184).
//
// Tables render one checkbox per row (`input[data-row-select]`, value = record
// key) plus a header select-all (`input[data-bulk-select-all]`). The selection
// lives in the bulk form's hidden transport (`input[data-bulk-ids]`), comma-
// delimited on both ends — `,a,b,`, empty when nothing is selected.
//
// On a live table that transport is bound to a signal, so the selection
// survives a shard rerun instead of dying with the swapped table: this script
// writes the wire and dispatches `change`, the runtime writes the signal, and
// the binding keeps the transport in step. Checkbox state is re-applied from
// the transport after every swap (a MutationObserver, because a swap replaces
// the boxes themselves and no `load`/`DOMContentLoaded` fires for it).
// Without a binding the same writes are inert, so static tables keep working
// with the transport as a plain hidden field.
//
// The destructive submit asks first (GH #184): `[data-bulk-confirm-trigger]`
// opens the alert dialog that lives inside the form, and the dialog's confirm
// button submits it. The confirm field rides in the dialog, and the handler
// refuses a POST without it — so this script is an affordance, never the
// safeguard. Opening the dialog reads the transport to report the selection
// size.
//
// Delimiters make membership exact: `,ab,` never matches `b`.
//
// A row the resource's per-record policy refuses (GH #235) renders its checkbox
// `disabled` with the reason as its accessible label: it is not a choice, so
// every selector here skips disabled boxes — select-all never checks one, the
// tri-state header never counts one, and one can never reach the transport. A
// refused key in the wire would only make the handler's all-or-nothing check
// turn the whole batch into a 403.
//
// Document-level delegation (like sidebar.js) so streamed/shard swaps that
// replace table markup need no re-installation. Scoped per table via
// `[data-table-root]` so multiple tables never cross-talk.
(() => {
function transportFor(root) {
  return root.querySelector('form[data-bulk-form] input[name="ids"]');
}

// The keys currently selected, in wire form.
function wireOf(value) {
  return (value || '')
    .split(',')
    .map((key) => key.trim())
    .filter((key) => key !== '');
}

function wireFrom(keys) {
  return keys.length === 0 ? '' : `,${keys.join(',')},`;
}

// Every row checkbox in the table, selectable or not.
function allBoxesIn(root) {
  return Array.from(root.querySelectorAll('input[data-row-select]'));
}

// The row checkboxes a user may check (GH #235): a row the policy denies delete
// renders `disabled`, and a disabled control is not part of the selection.
function boxesIn(root) {
  return allBoxesIn(root).filter((box) => !box.disabled);
}

// The new selection: what this page now has checked, plus the keys selected on
// other pages (those rows are not in the DOM, so only the transport knows them).
//
// Takes every box on the page, disabled ones included: a key this page renders
// as unselectable is decided here, not by a stale wire entry. Only the
// checked-and-selectable boxes contribute.
function selectionKeys(boxes, currentWire) {
  const pageKeys = new Set(boxes.map((box) => box.value));
  const kept = wireOf(currentWire).filter((key) => !pageKeys.has(key));
  const checked = boxes
    .filter((box) => box.checked && !box.disabled)
    .map((box) => box.value);
  return [...new Set([...kept, ...checked])];
}

function selectionFrom(root, currentWire) {
  return selectionKeys(allBoxesIn(root), currentWire);
}

// The tri-state header's state for the selectable boxes of a page: only rows a
// user may check are counted, so a page whose every allowed row is checked
// reads "all" even when a denied row sits among them.
function headerState(boxes) {
  const checked = boxes.filter((box) => box.checked);
  return {
    checked: boxes.length > 0 && checked.length === boxes.length,
    indeterminate: checked.length > 0 && checked.length < boxes.length,
  };
}

// Reflect `wire` into the DOM: row boxes and the tri-state header. Runs after a
// swap and after every change.
function sync(root, wire) {
  const keys = new Set(wireOf(wire));
  // Every box follows the wire, and a disabled one is forced unchecked: a keyed
  // swap can reuse the element a row had before its policy changed.
  allBoxesIn(root).forEach((box) => {
    box.checked = !box.disabled && keys.has(box.value);
  });
  // Tri-state header (GH #160): checked only when every selectable row is
  // checked, indeterminate on a partial selection — otherwise a select-all
  // followed by one uncheck leaves the header lying checked. A denied row is
  // not a row the header can speak for (GH #235).
  const all = root.querySelector('input[data-bulk-select-all]');
  if (all) {
    const state = headerState(boxesIn(root));
    all.checked = state.checked;
    all.indeterminate = state.indeterminate;
  }
}

function update(root) {
  const transport = transportFor(root);
  const wire = wireFrom(selectionFrom(root, transport ? transport.value : ''));
  if (transport && transport.value !== wire) {
    transport.value = wire;
    // The runtime writes the bound signal; a static table has nothing
    // listening and the transport is simply the form field it always was.
    transport.dispatchEvent(new Event('change', { bubbles: true }));
  }
  sync(root, wire);
}

// Everything below only makes sense with a document. It lives in a function so
// this file can also be `require`d by its Node unit test (`bulk.test.js`), which
// has no DOM: loading the script must not touch one.
function install() {
  document.addEventListener('change', (e) => {
    const all = e.target.closest('[data-bulk-select-all]');
    const row = e.target.closest('input[data-row-select]');
    if (!all && !row) return;
    const root = (all || row).closest('[data-table-root]');
    if (!root) return;
    if (all) {
      // Select-all reaches the selectable rows only (GH #235): a denied row's
      // box is disabled, so a click cannot put a refused key into the transport.
      boxesIn(root).forEach((box) => {
        box.checked = all.checked;
      });
    }
    update(root);
  });

  // The destructive confirm (GH #184). `type="button"`, so the dialog decides
  // when the form is submitted; the dialog's own confirm button is the ordinary
  // submit inside it.
  document.addEventListener('click', (e) => {
    const trigger = e.target.closest('[data-bulk-confirm-trigger]');
    if (!trigger) return;
    const form = trigger.closest('form[data-bulk-form]');
    const dialog = form && form.querySelector('[data-bulk-confirm-dialog]');
    if (!dialog) return;
    // Sync the transport from the live checkboxes before the dialog reports the
    // selection: the wire is authoritative, but a checkbox click that landed
    // mid-swap could still be unflushed.
    const root = form.closest('[data-table-root]');
    if (root) update(root);
    const description = dialog.querySelector('[data-bulk-confirm-description]');
    const count = wireOf(transportFor(root || form)?.value).length;
    if (description) {
      // A selection is required to delete anything, so an empty one says so
      // rather than opening a dialog whose Delete would only bounce back with an
      // error toast.
      if (count === 0) {
        description.textContent = 'Select at least one row first.';
      } else if (count === 1) {
        description.textContent = 'This action cannot be undone. 1 record is selected.';
      } else {
        description.textContent =
          `This action cannot be undone. ${count} records are selected.`;
      }
    }
    if (typeof dialog.showModal === 'function') dialog.showModal();
    else dialog.setAttribute('open', '');
  });

  // A swap replaces the table (and its checkboxes) without a page load, so
  // re-apply the selection whenever the table's markup changes.
  const observers = new WeakMap();
  function observe(root) {
    if (observers.has(root)) return;
    const observer = new MutationObserver(() => {
      const transport = transportFor(root);
      sync(root, transport ? transport.value : '');
    });
    observer.observe(root, { childList: true, subtree: true });
    observers.set(root, observer);
  }

  function watch(scope) {
    (scope || document).querySelectorAll('[data-table-root]').forEach((root) => {
      observe(root);
      const transport = transportFor(root);
      sync(root, transport ? transport.value : '');
    });
  }

  // A reload restores checkbox state before DOMContentLoaded; re-sync the
  // server-rendered state with the transport.
  document.addEventListener('DOMContentLoaded', () => watch());

  // The transport is kept current on every change; this is the belt-and-braces
  // pass for a submit that raced a swap, and it keeps the form the single owner
  // of the field the handler parses.
  document.addEventListener('submit', (e) => {
    const form = e.target.closest('form[data-bulk-form]');
    if (!form) return;
    const root = form.closest('[data-table-root]') || document;
    const transport = transportFor(root);
    if (!transport) return;
    transport.value = wireFrom(selectionFrom(root, transport.value));
  });

  // Streamed and swapped tables arrive after DOMContentLoaded: watch them too.
  new MutationObserver(() => watch()).observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
}

if (typeof document !== 'undefined') install();

// Exposed for the Node test in `assets/bulk.test.js` (there is no JS test
// runner in this workspace, and this file must stay a plain browser script
// loaded by `asset!`, so it cannot be an ES module). Guarded, so the browser
// branch is inert.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { headerState, selectionKeys, wireFrom, wireOf };
}
})();
