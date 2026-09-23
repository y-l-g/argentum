// Confirmed destructive submits, applied without a navigation (GH #234).
//
// A form marked `data-mutation-submit` — the row-delete confirm and the bulk
// confirm — POSTs through `fetch`, follows the 303, and applies the response
// in place:
//
// - the flash toast the handler set (`set_notification`) is inserted into the
//   shell's toaster, because following the redirect consumed the cookie that
//   carries it;
// - the table is refreshed through the seam that already owns it. A live
//   table renders a revision control inside its region (`[data-table-revision]`,
//   GH #234): writing it re-runs the `table_search` shard, which morphs and
//   re-hydrates the region with the live query state intact. A static table
//   has no shard and its region is inert markup, so the response's region
//   content replaces it wholesale.
//
// The mutation response is a whole list page, and the client never morphs it
// into the live document: markup inserted that way is never hydrated (the
// runtime hydrates only what its own units render), and the response renders
// the bare list URL while a live table's state lives in its signals. Driving
// the shard keeps one renderer — the server — for both halves.
//
// Failure paths are the browser's. A POST the server answers itself (4xx/5xx)
// wrote nothing — the handlers check before the write and roll back on any
// failure — so the form is handed back and the browser shows the same
// response a no-JS POST would. A request that never completes leaves the
// outcome unknown, so the page reloads instead of guessing.
//
// Without JavaScript none of this runs: the same form POSTs and 303s.
//
// Document-level delegation (like bulk.js) so a table a shard re-rendered
// needs no re-installation.
(() => {
// The record key a row-delete action URL names, or null when the URL is not
// one (`/admin/users/<key>/delete`, `delete_action_url`). Percent-encoded
// segments are decoded: the wire and the action carry the same raw key.
function deletedKey(action) {
  let url;
  try {
    url = new URL(action, 'http://localhost');
  } catch {
    return null;
  }
  const segments = url.pathname.split('/').filter((part) => part !== '');
  if (segments.length < 2 || segments[segments.length - 1] !== 'delete') {
    return null;
  }
  try {
    return decodeURIComponent(segments[segments.length - 2]);
  } catch {
    return null;
  }
}

// The keys a form's write removes: the batch a bulk form carried, or the one
// record a row-delete form's action names. Empty when the form says neither,
// so an unreadable target prunes nothing rather than clearing the selection.
function removedKeys(form, action) {
  const ids = form.querySelector('input[name="ids"]');
  if (ids) return wireOf(ids.value);
  const key = deletedKey(action);
  return key === null ? [] : [key];
}

// The keys in a `,a,b,`-delimited wire (bulk.js's format: the delimiters make
// membership exact, so `,ab,` never matches `b`).
function wireOf(value) {
  return (value || '')
    .split(',')
    .map((key) => key.trim())
    .filter((key) => key !== '');
}

function wireFrom(keys) {
  return keys.length === 0 ? '' : `,${keys.join(',')},`;
}

// The selection wire minus the keys this mutation removed. A bulk delete
// removes everything the form carried, so its wire empties; a row delete
// removes one key and leaves the rest of the selection standing.
function pruneWire(wire, removed) {
  const gone = new Set(removed);
  return wireFrom(wireOf(wire).filter((key) => !gone.has(key)));
}

// What a mutation response offers the live page: the table region to replace
// (a static table only) and the toast surfaces to mount.
//
// The region arrives inside a streamed swap envelope (`live!`): the response's
// own `[data-boundary="table"]` is the loading skeleton, and the rendered
// table rides in a `<template data-topcoat-swap>` at the end of the body that
// the page's inline `topcoat.swap` moves into the region at parse time. This
// reads the same content out of the parsed response, preferring the envelope
// and never taking the busy placeholder for a table.
function swapTargets(doc) {
  const streamed = Array.from(doc.querySelectorAll('template[data-topcoat-swap]'))
    .map((template) => template.content)
    .find((content) => content.querySelector('[data-boundary="table"]'));
  const root = streamed || doc;
  return {
    table: root.querySelector('[data-boundary="table"]:not([aria-busy])'),
    toasts: Array.from(
      doc.querySelectorAll('[data-sonner-toaster] > [data-sonner-toast]'),
    ),
  };
}

// The row a row-delete form targets: the control that opened the dialog
// carries this record's POST target (dialog.js copies it onto the form), and
// on a live page the dialog itself sits outside the table, so the trigger is
// where the row is still reachable from.
function rowOf(form) {
  const action = form.getAttribute('action');
  if (!action) return null;
  const trigger = Array.from(document.querySelectorAll('[data-row-delete-action]'))
    .find((el) => el.getAttribute('data-row-delete-action') === action);
  return trigger ? trigger.closest('tr') : null;
}

// The next revision token. Monotonic per page load, so the write always
// changes the signal — a same-value write is a no-op in the runtime.
let revisionToken = 0;

function bumpRevision(input) {
  revisionToken += 1;
  input.value = `r${revisionToken}`;
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

// Everything below only makes sense with a document. It lives in a function so
// this file can also be `require`d by its Node unit test, which has no DOM:
// loading the script must not touch one.
function install() {
  document.addEventListener('submit', (event) => {
    const form = event.target.closest('form[data-mutation-submit]');
    if (!form) return;
    // No action: the row dialog is retargeted by dialog.js from the control
    // that opens it, so this is markup the page cannot serve. The browser's
    // own submit is the honest fallback.
    const action = form.getAttribute('action');
    if (!action) return;
    event.preventDefault();
    send(form, action, event.submitter || form.querySelector('button[type="submit"]'));
  });
}

async function send(form, action, submitter) {
  // The confirm dialog the submit came from: the row-delete form lives inside
  // its dialog, while the bulk form carries its dialog as a child. Closing it
  // here is what returns focus to the page — a modal dialog left for the
  // response's markup to close (by dropping `open`) strands the document
  // inert, so nothing can be focused at all.
  const dialog = form.closest('dialog') || form.querySelector('dialog');
  const row = rowOf(form);
  const index = row ? Array.from(row.parentElement.children).indexOf(row) : -1;
  if (submitter) submitter.disabled = true;

  let response;
  try {
    response = await fetch(action, {
      method: 'POST',
      body: new FormData(form),
      credentials: 'same-origin',
      redirect: 'follow',
    });
  } catch {
    // The request never completed, so whether the write landed is unknown.
    // Reloading shows the server's truth instead of guessing at it.
    window.location.reload();
    return;
  }

  // The POST answered itself rather than redirecting: the server refused or
  // failed the write, and nothing was written. Hand the form back so the
  // browser shows that response exactly as a no-JS POST does.
  if (!response.redirected || !response.ok) {
    if (submitter) submitter.disabled = false;
    form.submit();
    return;
  }
  // The write is over, so the button that started it is usable again: on a
  // live page the dialog survives the rerun, and a disabled Delete would make
  // the next row's delete a dead click.
  if (submitter) submitter.disabled = false;

  const doc = new DOMParser().parseFromString(await response.text(), 'text/html');
  const { table, toasts } = swapTargets(doc);
  const region = document.querySelector('[data-boundary="table"]');
  const revision = document.querySelector('[data-table-revision]');
  // Nothing to update in place: a response the page cannot place is a page
  // the reader should be looking at.
  if (!region || (!revision && !table)) {
    window.location.assign(response.url);
    return;
  }
  // The selection the write removed, and the wire it leaves behind. Read now,
  // not at submit time: every submit listener has run by the time the response
  // lands, so this is the batch the form actually carried.
  const transport = document.querySelector('form[data-bulk-form] input[name="ids"]');
  const removed = removedKeys(form, action);
  const wire =
    transport && removed.length > 0
      ? pruneWire(transport.value, removed)
      : transport
        ? transport.value
        : '';

  insertToasts(toasts);
  dismiss(dialog);

  if (revision) {
    // The shard re-renders the region in place (fresh rows, same query,
    // hydrated markup) and cancels any rerun still in flight. The URL keeps
    // the state the table still holds; only the dialog's own parameters go.
    writeSelection(transport, wire);
    bumpRevision(revision);
    const url = new URL(window.location.href);
    url.searchParams.delete('delete');
    url.searchParams.delete('open');
    window.history.replaceState(window.history.state, '', url);
    afterRegionChange(region, () => focusAfter(index));
  } else {
    region.replaceChildren(
      ...Array.from(table.childNodes).map((node) => document.importNode(node, true)),
    );
    // The response's transport carries the server's empty wire: re-apply the
    // pruned one so a row delete does not clear the rest of the selection.
    writeSelection(
      document.querySelector('form[data-bulk-form] input[name="ids"]'),
      wire,
    );
    window.history.replaceState(window.history.state, '', response.url);
    focusAfter(index);
  }
}

// Write a selection wire into the bulk transport. The runtime writes the bound
// selection signal; a static table has nothing listening and bulk.js re-applies
// the boxes from the wire.
function writeSelection(transport, wire) {
  if (!transport || transport.value === wire) return;
  transport.value = wire;
  transport.dispatchEvent(new Event('change', { bubbles: true }));
}

// A toast surface is inserted, never morphed: morphing one re-syncs
// `data-mounted` from the response (`false`) on a surface notifications.js
// has already mounted, and the arming observer only sees added nodes — the
// toast would stay hidden. `notifications.js` arms what this inserts.
function insertToasts(toasts) {
  const toaster = document.querySelector('[data-sonner-toaster]');
  if (!toaster || toasts.length === 0) return;
  toaster.prepend(...toasts.map((toast) => document.importNode(toast, true)));
}

function dismiss(dialog) {
  if (dialog && dialog.open) dialog.close();
}

// Focus where the deleted row stood: the row that took its place, else the
// last row, else the bulk trigger. The table is the reader's context, and the
// control that opened the dialog is usually the element that just left. The
// table is looked up again here — both paths have replaced or morphed it by
// the time this runs.
function focusAfter(index) {
  const tbody = document.querySelector('[data-boundary="table"] tbody');
  const rows = tbody ? Array.from(tbody.children) : [];
  const target = index >= 0 ? rows[Math.min(index, rows.length - 1)] : null;
  const control =
    (target &&
      // The row's own Delete first (the control this flow is driven from),
      // then anything else a reader can land on. A disabled control — a row
      // the policy refuses — cannot take focus, so it does not count.
      (target.querySelector('a[data-row-delete-action]') ||
        target.querySelector(
          'a[href], button:not([disabled]), input:not([type="hidden"]):not([disabled])',
        ))) ||
    document.querySelector('[data-bulk-confirm-trigger]');
  if (control) control.focus();
}

// Runs `run` once the region's content has changed — the shard's re-render
// replaces the rows without a page event, and it is the re-render (not the
// response) that puts the new rows in the document. The timeout only drops the
// observer: a rerun that never lands must not focus anything later.
function afterRegionChange(region, run) {
  let done = false;
  const finish = (focus) => {
    if (done) return;
    done = true;
    observer.disconnect();
    window.clearTimeout(timer);
    if (focus) run();
  };
  const observer = new MutationObserver(() => finish(true));
  observer.observe(region, { childList: true, subtree: true });
  const timer = window.setTimeout(() => finish(false), 3000);
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`mutation-submit.test.js`). There is no JS
// test runner in this workspace and this file must stay a plain browser script
// loaded through `asset!`, so it cannot be an ES module. The guard keeps the
// browser branch inert.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { deletedKey, pruneWire, removedKeys, swapTargets, wireFrom, wireOf };
}
})();
