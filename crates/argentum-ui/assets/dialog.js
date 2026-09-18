// Dialog dismissal for SSR dialogs — the server renders `<dialog open>`, so
// the closed state is normally a navigation (Cancel/Delete are links). This
// script adds Escape, backdrop, and `[data-dialog-close]` dismissal without a
// reload.
//
// A dialog whose open state is URL-driven mirrors the dismissal back into the
// URL (`?open=false`, named by `data-dialog-open-param`) so a reload stays
// closed. A dialog driven by a runtime signal carries no such marker — its
// element's own `@close` handler keeps the signal in step — so dismissing it
// leaves the URL alone (GH #154 §3).
//
// Document-level delegation (like bulk.js) so a dialog that arrives in
// streamed or shard-swapped markup dismisses too — binding at
// DOMContentLoaded missed anything the server rendered later.
function dismissDialog(dialog) {
  if (!dialog.open) return;
  dialog.close();
  const param = dialog.dataset.dialogOpenParam;
  if (!param) return;
  const url = new URL(window.location.href);
  url.searchParams.set(param, 'false');
  window.history.pushState(window.history.state, '', url);
}

document.addEventListener('click', (e) => {
  const dialog = e.target.closest('dialog[open]');
  if (!dialog) return;
  // The overlay is the <dialog> itself; a click on it (not the panel inside)
  // is the backdrop.
  if (e.target === dialog) {
    dismissDialog(dialog);
    return;
  }
  if (e.target.closest('[data-dialog-close]')) {
    e.preventDefault();
    dismissDialog(dialog);
  }
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
