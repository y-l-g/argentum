// Unit test for the row-delete trigger wiring in `dialog.js` (GH #233).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the function through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test crates/argentum-ui/assets/dialog.test.js
//
// What it protects: the table renders one row-delete dialog, closed, and each
// row Delete control names it and carries that row's POST target. The dialog
// must be pointed at the clicked record before it opens — a stale action would
// delete the row the previous click named — and a control the page cannot
// serve (no dialog, no target) must fall through to its `?delete=` href rather
// than open a dialog whose Delete posts somewhere unintended. The click wiring
// itself is driven through a document stand-in, so "opens without navigating"
// is a test and not a reading of the listener.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./dialog.js');
const { openDeleteDialog } = require(SCRIPT);

// A row Delete control, as the DOM hands it over: the dialog it names and the
// record's POST target.
const triggerOf = (dialogId, action) => ({
  getAttribute: (name) =>
    name === 'data-row-delete-trigger'
      ? dialogId
      : name === 'data-row-delete-action'
        ? action
        : null,
});

// The dialog and its form, with the calls the wiring makes recorded.
const dialogOf = ({ open = false } = {}) => {
  const calls = { opened: 0, action: null, openAttr: null };
  const form = { setAttribute: (name, value) => (calls.action = [name, value]) };
  const dialog = {
    open,
    calls,
    querySelector: (selector) =>
      selector === '[data-row-delete-form]' ? form : null,
    showModal: () => calls.opened++,
    setAttribute: (name, value) => (calls.openAttr = [name, value]),
  };
  return dialog;
};

// A document holding one dialog, reachable by the id its controls name.
const docOf = (dialog) => ({
  getElementById: (id) => (id === 'admin-users-delete-dialog' ? dialog : null),
});

// --- a document stand-in -----------------------------------------------------

// `dialog.js` is a plain browser script: `install()` reads `document` from the
// global scope and every handler is document-delegated, so the stand-in has to
// be in place before the script is required and stay there while its listeners
// run. It is only as wide as the script needs.
function standInDocument(dialog) {
  const byType = new Map();
  return {
    addEventListener(type, handler) {
      if (!byType.has(type)) byType.set(type, []);
      byType.get(type).push(handler);
    },
    getElementById: (id) => (id === 'admin-users-delete-dialog' ? dialog : null),
    querySelector: () => null,
    // Every listener for `type`, in registration order: firing them all is what
    // a browser does for one event.
    listeners(type) {
      return byType.get(type) || [];
    },
  };
}

// Load a fresh copy of the script against `document`, run the case, and drop
// the stand-in: a fresh copy re-runs `install()`, so each case gets its own
// listener set.
function withDocument(standIn, run) {
  global.document = standIn;
  delete require.cache[SCRIPT];
  try {
    require(SCRIPT);
    run();
  } finally {
    delete global.document;
  }
}

// A click on a row Delete control, as the browser hands it to the listener: the
// target answers the trigger selector and nothing else.
function triggerClick(trigger) {
  return {
    prevented: false,
    target: {
      closest: (selector) =>
        selector === '[data-row-delete-trigger]' ? trigger : null,
    },
    preventDefault() {
      this.prevented = true;
    },
  };
}

test('the trigger points the dialog at its record before opening it', () => {
  const dialog = dialogOf();
  const trigger = triggerOf(
    'admin-users-delete-dialog',
    '/admin/users/ada/delete',
  );
  const opened = openDeleteDialog(trigger, docOf(dialog));
  assert.equal(opened, dialog, 'the named dialog is the one that opens');
  assert.deepEqual(
    dialog.calls.action,
    ['action', '/admin/users/ada/delete'],
    'the form posts to the clicked record',
  );
  assert.equal(dialog.calls.opened, 1, 'the dialog is shown modally');
});

test('a second row retargets the same dialog', () => {
  // One dialog per table: the action is rewritten per click, so the confirm
  // never posts the record the previous click named.
  const dialog = dialogOf();
  const doc = docOf(dialog);
  const row = (key) =>
    triggerOf('admin-users-delete-dialog', `/admin/users/${key}/delete`);
  openDeleteDialog(row('ada'), doc);
  openDeleteDialog(row('ken'), doc);
  assert.deepEqual(dialog.calls.action, ['action', '/admin/users/ken/delete']);
});

test('a trigger naming a dialog the page lacks opens nothing', () => {
  // A chromeless table renders no dialog: the caller must leave the click to
  // the link, whose `?delete=` href renders the dialog server-side.
  const opened = openDeleteDialog(
    triggerOf('other-table-delete-dialog', '/admin/users/ada/delete'),
    docOf(dialogOf()),
  );
  assert.equal(opened, null);
});

test('a trigger without a POST target opens nothing', () => {
  // Both attributes come from the one policy decision, so this is malformed
  // markup — opening here would submit the form to whatever action it already
  // carried. The link's own navigation is the honest fallback.
  const dialog = dialogOf();
  const opened = openDeleteDialog(
    triggerOf('admin-users-delete-dialog', null),
    docOf(dialog),
  );
  assert.equal(opened, null);
  assert.equal(dialog.calls.opened, 0, 'nothing opens');
  assert.equal(dialog.calls.action, null, 'the form keeps the action it had');
});

test('a dialog with no showModal still opens', () => {
  // `showModal` is the modal path; the fallback is the attribute the server
  // itself renders for a URL-driven dialog.
  const dialog = dialogOf();
  delete dialog.showModal;
  const opened = openDeleteDialog(
    triggerOf('admin-users-delete-dialog', '/admin/users/ada/delete'),
    docOf(dialog),
  );
  assert.equal(opened, dialog);
  assert.deepEqual(dialog.calls.openAttr, ['open', '']);
});

test('an already open dialog is retargeted, never re-shown', () => {
  // Defensive: the open dialog covers the row controls, so the browser cannot
  // produce this click — but `showModal` throws on an open dialog, so a trigger
  // that does arrive must retarget the form without touching the open state.
  const dialog = dialogOf({ open: true });
  const opened = openDeleteDialog(
    triggerOf('admin-users-delete-dialog', '/admin/users/ken/delete'),
    docOf(dialog),
  );
  assert.equal(opened, dialog, 'the caller still swallows the navigation');
  assert.deepEqual(dialog.calls.action, ['action', '/admin/users/ken/delete']);
  assert.equal(dialog.calls.opened, 0, 'the open dialog is not shown again');
});

test('a click that opens the dialog swallows the link navigation', () => {
  // The central claim: one navigation per delete. `install()` answers the click
  // in place, so the control's `?delete=` href is never followed — and the
  // handler, not the pure function, is what has to prevent it.
  const dialog = dialogOf();
  withDocument(standInDocument(dialog), () => {
    const event = triggerClick(
      triggerOf('admin-users-delete-dialog', '/admin/users/ada/delete'),
    );
    // Every `click` listener, in registration order, as the browser fires them.
    global.document.listeners('click').forEach((handler) => handler(event));
    assert.equal(event.prevented, true, 'the click must not navigate');
    assert.equal(dialog.calls.opened, 1, 'the dialog is the answer');
  });
});

test('a click the page cannot serve is left to the link', () => {
  // No dialog on the page: the click falls through to the href, which renders
  // the dialog server-side. Preventing it here would leave the row with no
  // delete at all.
  withDocument(standInDocument(null), () => {
    const event = triggerClick(
      triggerOf('admin-users-delete-dialog', '/admin/users/ada/delete'),
    );
    global.document.listeners('click').forEach((handler) => handler(event));
    assert.equal(event.prevented, false, 'the link opens the fallback');
  });
});
