// Unit test for the row-delete trigger wiring and the dismissal rules in
// `dialog.js`.
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the function through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test crates/tablo-ui/assets/dialog.test.js
//
// What it protects: the table renders one row-delete dialog, closed, and each
// row Delete control names it and carries that row's POST target. The dialog
// must be pointed at the clicked record before it opens — a stale action would
// delete the row the previous click named — and a control the page cannot
// serve (no dialog, no target) must fall through to its `?delete=` href rather
// than open a dialog whose Delete posts somewhere unintended. The click wiring
// itself is driven through a document stand-in, so "opens without navigating"
// is a test and not a reading of the listener.
//
// The dismissal cases drive the same stand-in: an alert dialog keeps the
// backdrop from standing in for an answer, a dialog mid-mutation ignores every
// dismissal, and the `?open=false` bookkeeping the paths that work already do
// stays in place.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./dialog.js');
const { openDeleteDialog } = require(SCRIPT);

const { listenerDocument } = require('./test-dom');

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

// The dialog and its form, with the calls the wiring makes recorded. `role`
// is what the server renders (an `alert_dialog` carries `alertdialog`);
// `busy` is the marker `mutation-submit.js` sets while a write is in flight.
const dialogOf = ({
  open = false,
  role = null,
  busy = false,
  openParam = null,
} = {}) => {
  const calls = { opened: 0, closed: 0, action: null, openAttr: null };
  const attrs = role ? { role } : {};
  const form = { setAttribute: (name, value) => (calls.action = [name, value]) };
  const dialog = {
    open,
    calls,
    dataset: {},
    querySelector: (selector) =>
      selector === '[data-row-delete-form]' ? form : null,
    getAttribute: (name) => (name in attrs ? attrs[name] : null),
    setAttribute: (name, value) => (calls.openAttr = [name, value]),
    showModal: () => calls.opened++,
    close() {
      calls.closed += 1;
      this.open = false;
    },
    // The delegated handlers resolve the dialog from the event target.
    closest: (selector) => (selector === 'dialog[open]' ? dialog : null),
  };
  if (busy) dialog.dataset.dialogBusy = 'true';
  if (openParam) dialog.dataset.dialogOpenParam = openParam;
  return dialog;
};

// A document holding one dialog, reachable by the id its controls name.
const docOf = (dialog) => ({
  getElementById: (id) => (id === 'admin-users-delete-dialog' ? dialog : null),
});

// --- a document stand-in -----------------------------------------------------

// `dialog.js` is a plain browser script: `install()` reads `document` and
// `window` from the global scope and every handler is document-delegated, so
// the stand-ins have to be in place before the script is required and stay
// there while its listeners run. It is only as wide as the script needs.
function standInDocument(dialog) {
  return listenerDocument({
    getElementById: (id) => (id === 'admin-users-delete-dialog' ? dialog : null),
    // Escape reads the open dialog off the document.
    querySelector: (selector) =>
      selector === 'dialog[open]' && dialog && dialog.open ? dialog : null,
  });
}

// Load a fresh copy of the script against `document`, run the case, and drop
// the stand-ins: a fresh copy re-runs `install()`, so each case gets its own
// listener set. `pushed` records the `?open=false` history the URL-driven
// dismissal writes.
function withDocument(standIn, run) {
  const pushed = [];
  const realWindow = global.window;
  global.document = standIn;
  global.window = {
    location: { href: 'http://localhost/admin/users' },
    history: {
      state: null,
      pushState(state, title, url) {
        pushed.push(String(url));
      },
    },
  };
  delete require.cache[SCRIPT];
  try {
    require(SCRIPT);
    run({ pushed });
  } finally {
    delete global.document;
    global.window = realWindow;
  }
}

// A click whose target is the <dialog> itself: the overlay, not the panel.
function backdropClick(dialog) {
  return { target: dialog };
}

// A click on a control inside the dialog. `close` marks it
// `[data-dialog-close]`; `link` marks it a real navigation, which the script
// leaves to the browser.
function clickInside(dialog, { close = false, link = false } = {}) {
  const target = {
    closest: (selector) => {
      if (selector === 'dialog[open]') return dialog;
      if (selector === '[data-dialog-close]') return close ? target : null;
      if (selector === 'a[href]') return link ? target : null;
      return null;
    },
  };
  return { target };
}

// Fire every `type` listener the browser would, in registration order.
function fire(type, event) {
  global.document.listeners(type).forEach((handler) => handler(event));
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

// --- GH #293: the alert backdrop and the in-flight dialog --------------------

test('a backdrop click does not dismiss an alert dialog', () => {
  // An alert dialog asks for an answer, and the backdrop is not one: it stays
  // until Escape or one of its own controls closes it.
  const dialog = dialogOf({ open: true, role: 'alertdialog' });
  withDocument(standInDocument(dialog), () => {
    fire('click', backdropClick(dialog));
    assert.equal(dialog.calls.closed, 0, 'the alert dialog waits for an answer');
  });
});

test('a backdrop click still dismisses a plain dialog', () => {
  // The alert rule is the exception, not a blanket refusal to dismiss.
  const dialog = dialogOf({ open: true });
  withDocument(standInDocument(dialog), () => {
    fire('click', backdropClick(dialog));
    assert.equal(dialog.calls.closed, 1, 'a plain dialog dismisses on the backdrop');
  });
});

test('a dialog mid-mutation ignores a backdrop click', () => {
  // The write owns the dialog until its response lands: closing it here would
  // let a later click open the same dialog for another record, and the
  // response would then close that one.
  const dialog = dialogOf({ open: true, busy: true });
  withDocument(standInDocument(dialog), () => {
    fire('click', backdropClick(dialog));
    assert.equal(dialog.calls.closed, 0, 'the write still owns the dialog');
  });
});

test('a dialog mid-mutation ignores Escape', () => {
  const dialog = dialogOf({ open: true, busy: true });
  withDocument(standInDocument(dialog), () => {
    fire('keydown', { key: 'Escape' });
    assert.equal(dialog.calls.closed, 0, 'Escape cannot strand the write either');
  });
});

test('an alert dialog closes through its own control', () => {
  const dialog = dialogOf({ open: true, role: 'alertdialog' });
  withDocument(standInDocument(dialog), () => {
    fire('click', clickInside(dialog, { close: true }));
    assert.equal(dialog.calls.closed, 1, 'the Cancel control is the answer');
  });
});

test('a dismissed URL-driven dialog still mirrors ?open=false', () => {
  // The bookkeeping the paths that work already do: a dialog whose open state
  // is in the URL writes the dismissal back so a reload stays closed.
  const dialog = dialogOf({ open: true, role: 'alertdialog', openParam: 'open' });
  withDocument(standInDocument(dialog), ({ pushed }) => {
    fire('click', clickInside(dialog, { close: true }));
    assert.equal(dialog.calls.closed, 1);
    assert.deepEqual(pushed, ['http://localhost/admin/users?open=false']);
  });
});
