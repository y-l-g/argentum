// Unit test for the confirmed-mutation wiring in `mutation-submit.js` (GH #234).
//
// There is no JS test runner in this workspace — the assets are plain browser
// scripts loaded through `asset!` — so this runs on Node's built-in runner and
// reaches the functions through the guarded `module.exports` at the bottom of
// the script:
//
//     node --test crates/argentum-ui/assets/mutation-submit.test.js
//
// What it protects: which region a mutation response hands over (the rendered
// table inside the streamed swap envelope, never the loading skeleton the
// response also carries), which toast surfaces mount, which keys a write
// removed from the bulk selection, which submits this script answers at all —
// a form the page cannot serve must keep the browser's own submit — and what
// it does with a response the server answered itself: the fetched page is
// shown in place, and the mutation is not posted again (GH #293).
//
// The DOM half of the script (the shard re-render, `DOMParser`) has no
// stand-in here; it is verified against a running panel instead. The fetch
// stand-in below covers the failure branch, which is where the re-send lived.

const test = require('node:test');
const assert = require('node:assert/strict');

const SCRIPT = require.resolve('./mutation-submit.js');
const {
  deletedKey,
  pruneWire,
  removedKeys,
  swapTargets,
  tableRootFor,
  wireFrom,
  wireOf,
} = require(SCRIPT);

// --- the record a row-delete action names -----------------------------------

test('a row-delete action names its record key', () => {
  assert.equal(
    deletedKey('/admin/users/01a0cf6a-7885-75e6-8230-74b677786c1f/delete'),
    '01a0cf6a-7885-75e6-8230-74b677786c1f',
  );
});

test('a percent-encoded key is decoded to the wire spelling', () => {
  // The wire and the action carry the same record key: the checkbox value is
  // the raw PK, the URL segment is the encoded one. Pruning compares raw keys,
  // so the action's segment has to come back decoded.
  assert.equal(deletedKey('/admin/users/a%20b%2Fc/delete'), 'a b/c');
});

test('a URL that is not a row-delete action names no record', () => {
  // The bulk-delete route and anything else must not prune a key.
  assert.equal(deletedKey('/admin/users/bulk-delete'), null);
  assert.equal(deletedKey('/admin/users'), null);
  assert.equal(deletedKey(''), null);
});

// --- the keys a write removed -----------------------------------------------

// A form stand-in carrying the fields the decision reads.
const formWith = (fields) => ({
  querySelector: (selector) => {
    const name = /name="([^"]+)"/.exec(selector);
    return name && name[1] in fields ? fields[name[1]] : null;
  },
});

test('a bulk form removes the batch it carried', () => {
  // The transport is the form's `ids` field: exactly the keys the handler
  // deletes, so the wire empties after the batch lands.
  const form = formWith({ ids: { value: ',a,b,' } });
  assert.deepEqual(removedKeys(form, '/admin/users/bulk-delete'), ['a', 'b']);
});

test('a row form removes the one record its action names', () => {
  const form = formWith({});
  assert.deepEqual(removedKeys(form, '/admin/users/ada/delete'), ['ada']);
});

test('a form naming neither removes nothing', () => {
  // Pruning nothing keeps a selection the write did not touch; guessing here
  // would clear a selection the reader still holds.
  assert.deepEqual(removedKeys(formWith({}), '/admin/users/bulk-delete'), []);
});

// --- which table a form belongs to ------------------------------------------

// A node stand-in whose `closest` answers the selectors a case gives it.
const closestOf = (answers) => ({
  closest: (selector) => answers[selector] || null,
});

test('a form inside a table belongs to that table', () => {
  const own = closestOf({});
  const form = closestOf({ '[data-table-root]': own });
  assert.equal(tableRootFor(form, '/admin/users/ada/delete'), own);
});

test('a row confirm outside every table finds its table through its control', () => {
  // The row dialog is page-owned and sits outside the region, so the form has
  // no table ancestor: the control that opened it (carrying the same POST
  // target) is what names the table.
  const root = closestOf({});
  const trigger = {
    getAttribute: (name) =>
      name === 'data-row-delete-action' ? '/admin/users/ada/delete' : null,
    closest: (selector) => (selector === '[data-table-root]' ? root : null),
  };
  global.document = { querySelectorAll: () => [trigger] };
  try {
    const form = closestOf({});
    assert.equal(tableRootFor(form, '/admin/users/ada/delete'), root);
  } finally {
    delete global.document;
  }
});

test('a form whose control is gone belongs to no table', () => {
  // A stale action (the row was re-rendered away) must not fall back to the
  // document: the client then leaves the page alone instead of touching the
  // wrong table.
  global.document = { querySelectorAll: () => [] };
  try {
    assert.equal(tableRootFor(closestOf({}), '/admin/users/ada/delete'), null);
  } finally {
    delete global.document;
  }
});

// --- the selection wire -----------------------------------------------------

test('pruning drops the removed keys and keeps the rest', () => {
  assert.equal(pruneWire(',a,b,c,', ['b']), ',a,c,');
  assert.equal(pruneWire(',a,b,', ['a', 'b']), '');
  assert.equal(pruneWire('', ['a']), '');
});

test('pruning keeps keys selected on another page', () => {
  // The wire carries selections the page does not render (bulk.js keeps them
  // across pages); a delete must not drop those.
  assert.equal(pruneWire(',page2,page3,', ['page2']), ',page3,');
});

test('wire membership is exact', () => {
  // `,ab,` never matches `b`: the delimiters are what make the comparison a
  // membership test rather than a substring one.
  assert.deepEqual(wireOf(',ab,'), ['ab']);
  assert.equal(pruneWire(',ab,', ['b']), ',ab,');
  assert.equal(wireFrom(['a', 'b']), ',a,b,');
  assert.equal(wireFrom([]), '');
});

// --- what the response hands over -------------------------------------------

// A node stand-in: `querySelector` answers only the selectors the case uses.
const nodeOf = (selectors) => ({
  querySelector: (selector) => selectors[selector] || null,
});

// A node holding a rendered region. Two selectors reach it: the envelope is
// identified by carrying the region at all, and the region itself is taken
// with the busy placeholder excluded.
const regionNode = (table) =>
  nodeOf({
    '[data-boundary="table"]': table,
    '[data-boundary="table"]:not([aria-busy])': table,
  });

// A document stand-in holding the pieces `swapTargets` reads.
const docOf = ({ templates = [], toasts = [], region = null } = {}) => ({
  querySelectorAll: (selector) => {
    if (selector === 'template[data-topcoat-swap]') return templates;
    if (selector === '[data-sonner-toaster] > [data-sonner-toast]') return toasts;
    return [];
  },
  querySelector: (selector) =>
    selector === '[data-boundary="table"]:not([aria-busy])' ? region : null,
});

test('the streamed envelope wins over the skeleton the response also carries', () => {
  // A streamed list response renders the skeleton inside the region and ships
  // the table in a `<template data-topcoat-swap>` at the end of the body. The
  // skeleton is the same `[data-boundary="table"]`, so taking the document's
  // own region would replace a table with a loading placeholder.
  const table = nodeOf({});
  const skeleton = nodeOf({});
  const targets = swapTargets(
    docOf({
      templates: [{ content: nodeOf({}) }, { content: regionNode(table) }],
      region: skeleton,
    }),
  );
  assert.equal(targets.table, table);
});

test('a response with no envelope hands over its own rendered region', () => {
  const table = nodeOf({});
  const targets = swapTargets(docOf({ region: table }));
  assert.equal(targets.table, table);
});

test('a response offering no rendered region hands over nothing', () => {
  // The client then navigates instead of replacing the table with a skeleton.
  assert.equal(swapTargets(docOf({})).table, null);
});

test('every toast surface the response carries is handed over', () => {
  const toasts = [nodeOf({}), nodeOf({})];
  assert.deepEqual(swapTargets(docOf({ toasts })).toasts, toasts);
});

// --- which submits this script answers --------------------------------------

// `install()` reads `document` and `window` from the global scope and every
// listener is document-delegated, so the stand-ins have to be in place before
// the script is required and stay there while its listeners run. `written`
// records the page `showResponse` replaces the document with.
function standInDocument() {
  const byType = new Map();
  return {
    written: [],
    addEventListener(type, handler) {
      if (!byType.has(type)) byType.set(type, []);
      byType.get(type).push(handler);
    },
    listeners(type) {
      return byType.get(type) || [];
    },
    // The wiring reads the page before it posts: the row a delete came from
    // and the dialog it was confirmed in.
    querySelectorAll: () => [],
    querySelector: () => null,
    // `showResponse` renders the fetched answer into the document.
    open() {},
    write(html) {
      this.written.push(html);
    },
    close() {},
  };
}

// A form stand-in: the marker answers the listener's `closest`, the action is
// what the case gives it, and `dialog` is the confirm dialog the row form
// lives in. `new FormData(form)` cannot serialize this, which is the point —
// the post is never reached, only the decision is under test.
const submitForm = ({
  marked = true,
  action = '/admin/users/ada/delete',
  dialog = null,
} = {}) => {
  const form = {
    getAttribute: (name) => (name === 'action' ? action : null),
    matches: () => marked,
    closest: (selector) => {
      if (selector === 'dialog') return dialog;
      return marked && selector === 'form[data-mutation-submit]' ? form : null;
    },
    querySelector: (selector) => (selector === 'dialog' ? dialog : null),
  };
  return form;
};

// Load a fresh copy of the script against the stand-ins, run the case, and
// drop them: a fresh copy re-runs `install()`, so each case gets its own
// listener set. An async case keeps the globals until it settles, because
// `send` reads them after the fetch resolves.
function withGlobals(run) {
  const document = standInDocument();
  const calls = { reloaded: 0, assigned: [] };
  global.document = document;
  global.window = {
    location: {
      reload: () => calls.reloaded++,
      assign: (url) => calls.assigned.push(url),
      href: 'http://localhost/admin/users',
    },
    history: { state: null, replaceState() {} },
    setTimeout: () => 0,
    clearTimeout() {},
  };
  delete require.cache[SCRIPT];
  const drop = () => {
    delete global.document;
    delete global.window;
  };
  let result;
  try {
    require(SCRIPT);
    result = run({ document, calls });
  } catch (error) {
    drop();
    throw error;
  }
  if (result && typeof result.then === 'function') {
    return result.finally(drop);
  }
  drop();
  return result;
}

// A submit event as the browser hands it to the listener.
function submitEvent(form) {
  return {
    prevented: false,
    target: form,
    submitter: null,
    preventDefault() {
      this.prevented = true;
    },
  };
}

test('a marked form with a target is answered in place', () => {
  withGlobals(({ document }) => {
    const form = submitForm();
    const event = submitEvent(form);
    document.listeners('submit').forEach((handler) => handler(event));
    assert.equal(event.prevented, true, 'the navigation must not happen');
  });
});

test('a form the page cannot serve keeps the browser submit', () => {
  // The row dialog is retargeted from the control that opens it, so an
  // actionless form is markup the page cannot post: swallowing it would leave
  // the row with no delete at all.
  withGlobals(({ document }) => {
    const form = submitForm({ action: null });
    const event = submitEvent(form);
    document.listeners('submit').forEach((handler) => handler(event));
    assert.equal(event.prevented, false, 'the browser posts it');
  });
});

test('a form without the marker is left alone', () => {
  // Every other form on the page — create, edit, login, the no-JS search —
  // is not this script's business.
  withGlobals(({ document }) => {
    const form = submitForm({ marked: false });
    const event = submitEvent(form);
    document.listeners('submit').forEach((handler) => handler(event));
    assert.equal(event.prevented, false, 'the browser posts it');
  });
});

// --- a response the server answered itself (GH #293) ------------------------

// A form stand-in that records a browser submit, so "the delete is not sent
// again" is an observation rather than a reading of the listener.
function recordableForm(options) {
  const form = submitForm(options);
  form.submits = 0;
  form.submit = () => {
    form.submits += 1;
  };
  return form;
}

// `send` reads `FormData` and `fetch` from the global scope. The fetch answers
// with a response that never redirected: the server answered the POST itself.
async function withServerAnswer(answer, run) {
  return withGlobals(async (context) => {
    const realFormData = global.FormData;
    const realFetch = global.fetch;
    global.FormData = class {
      constructor(form) {
        this.form = form;
      }
    };
    global.fetch = async () => answer;
    try {
      return await run(context);
    } finally {
      global.FormData = realFormData;
      global.fetch = realFetch;
    }
  });
}

test('a server-answered failure is shown, not posted again', async () => {
  // The server can commit the write and then fail — a panic in `after_commit`
  // answers 500 — so the response does not mean "nothing happened". The page
  // the no-JS POST would have rendered is shown in place, and the delete is
  // never sent a second time.
  await withServerAnswer(
    {
      redirected: false,
      ok: false,
      status: 500,
      text: async () => '<!doctype html><p>the write failed</p>',
    },
    async ({ document }) => {
      const form = recordableForm();
      const event = submitEvent(form);
      document.listeners('submit').forEach((handler) => handler(event));
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(form.submits, 0, 'the committed delete must not be re-sent');
      assert.deepEqual(
        document.written,
        ['<!doctype html><p>the write failed</p>'],
        'the server answer is shown where the browser would have shown it',
      );
    },
  );
});

test('a refused submit shows the response without repeating the request', async () => {
  // A 4xx is the server refusing the write, but the client still does not know
  // the record is untouched, so it applies the same rule: show, never re-send.
  await withServerAnswer(
    {
      redirected: false,
      ok: false,
      status: 403,
      text: async () => '<!doctype html><p>not allowed</p>',
    },
    async ({ document }) => {
      const form = recordableForm();
      const event = submitEvent(form);
      document.listeners('submit').forEach((handler) => handler(event));
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(form.submits, 0, 'the refused delete is not posted again');
      assert.deepEqual(document.written, ['<!doctype html><p>not allowed</p>']);
    },
  );
});

test('the confirm dialog is held while the mutation is in flight', async () => {
  // `dialog.js` refuses to dismiss a dialog carrying this marker, so the write
  // owns it until its response is in hand (GH #293).
  await withServerAnswer(
    {
      redirected: false,
      ok: false,
      status: 500,
      text: async () => '<!doctype html><p>the write failed</p>',
    },
    async ({ document }) => {
      const dialog = { dataset: {} };
      const form = recordableForm({ dialog });
      const event = submitEvent(form);
      document.listeners('submit').forEach((handler) => handler(event));
      assert.equal(
        dialog.dataset.dialogBusy,
        'true',
        'the dialog belongs to the write while it is outstanding',
      );
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(
        dialog.dataset.dialogBusy,
        undefined,
        'the response hands the dialog back',
      );
    },
  );
});
