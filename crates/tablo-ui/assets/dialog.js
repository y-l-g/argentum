// Dialog behavior for SSR dialogs.
//
// Opening: a table renders one row-delete dialog, closed, and every row Delete
// control names it (`data-row-delete-trigger`) and carries the record's POST
// target (`data-row-delete-action`). The control keeps its `?delete=<key>`
// href, so this upgrades that navigation into an in-place open and a page
// without the script keeps the server-open fallback.
//
// Dismissal: the server renders `<dialog open>` for a URL-driven dialog, so
// the closed state is normally a navigation. This script adds Escape,
// backdrop, and `[data-dialog-close]` dismissal without a reload.
//
// Two dialogs are not dismissed by every path. An alert dialog asks for an
// answer, so a backdrop click is not one and never closes it — Escape and
// `[data-dialog-close]` stay the ways out. A dialog whose mutation is in
// flight (`data-dialog-busy`, set by `mutation-submit.js`) is held until the
// response lands: dismissing it would strand the write, and the response would
// then close whatever dialog a later click opened in its place.
//
// A dialog whose open state is URL-driven mirrors the dismissal back into the
// URL (`?open=false`, named by `data-dialog-open-param`) so a reload stays
// closed. A dialog driven by a runtime signal carries no such marker — its
// element's own `@close` handler keeps the signal in step — so dismissing it
// leaves the URL alone (§3); the bulk confirm and a dialog a
// row control opens have no URL state to close, so they carry none.
//
// Document-level delegation (like bulk.js) so a dialog that arrives in
// streamed or shard-swapped markup dismisses too — binding at
// DOMContentLoaded missed anything the server rendered later.
function dismissDialog(dialog) {
  if (!dialog.open) return;
  // A mutation in flight owns the dialog.
  if (dialog.dataset.dialogBusy === 'true') return;
  dialog.close();
  const param = dialog.dataset.dialogOpenParam;
  if (!param) return;
  const url = new URL(window.location.href);
  url.searchParams.set(param, 'false');
  window.history.pushState(window.history.state, '', url);
}

// Point a table's row-delete dialog at one record and open it.
//
// The trigger names its dialog and carries the POST target, both rendered by
// the server from the per-record policy decision that decided the row gets a
// Delete control at all: the browser never points the dialog at a
// record the policy refused. One dialog per table, so the trigger's value is
// the id of the dialog that belongs to its own table.
//
// Returns the dialog the click is answered by, or null when there is none — no
// dialog on the page (a chromeless table) or no record to point it at. The
// caller then leaves the click to the link's `?delete=` href, which opens the
// same dialog server-side with the action already set.
function openDeleteDialog(trigger, doc) {
  const id = trigger.getAttribute('data-row-delete-trigger');
  const action = trigger.getAttribute('data-row-delete-action');
  const dialog = id && action ? doc.getElementById(id) : null;
  if (!dialog) return null;
  const form = dialog.querySelector('[data-row-delete-form]');
  if (form) form.setAttribute('action', action);
  // Defensive: an open dialog covers the row controls, so a click cannot reach
  // one — but `showModal` throws on an already open dialog, so a trigger that
  // does arrive here only retargets the form.
  if (dialog.open) return dialog;
  if (typeof dialog.showModal === 'function') dialog.showModal();
  else dialog.setAttribute('open', '');
  return dialog;
}

// Everything below only makes sense with a document. It lives in a function so
// this file can also be `require`d by its Node unit test (`dialog.test.js`),
// which has no DOM: loading the script must not touch one.
function install() {
  document.addEventListener('click', (e) => {
    const dialog = e.target.closest('dialog[open]');
    if (!dialog) return;
    // The overlay is the <dialog> itself; a click on it (not the panel inside)
    // is the backdrop.
    if (e.target === dialog) {
      // An alert dialog asks for an answer, so the backdrop is not one: it
      // stays until Escape or a `[data-dialog-close]` control answers it
      // . A dialog mid-mutation is held by `dismissDialog` either way.
      if (dialog.getAttribute('role') !== 'alertdialog') {
        dismissDialog(dialog);
      }
      return;
    }
    if (e.target.closest('[data-dialog-close]')) {
      // A `data-dialog-close` *link* is left to navigate on its own; a
      // *button* has nothing to navigate to, so it is dismissed here instead —
      // the bulk-delete confirm's Cancel and the row-delete
      // dialog's.
      if (!e.target.closest('a[href]')) {
        dismissDialog(dialog);
      }
    }
  });

  document.addEventListener('click', (e) => {
    const trigger = e.target.closest('[data-row-delete-trigger]');
    if (!trigger) return;
    // A page that carries the dialog answers the click in place; without one
    // the link navigates to `?delete=<key>`, which renders the same dialog
    // open with the action already set.
    if (openDeleteDialog(trigger, document)) e.preventDefault();
  });

  // Escape. Modal dialogs (`showModal`) fire `cancel` on their own, but SSR
  // dialogs are non-modal `<dialog open>` and get no such event, so dismiss
  // straight from the key. A modal dialog closes natively as well; closing an
  // already closed dialog is a no-op.
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    const dialog = document.querySelector('dialog[open]');
    if (dialog) dismissDialog(dialog);
  });
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`dialog.test.js`); see `bulk.js` for the
// guard.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { openDeleteDialog };
}
