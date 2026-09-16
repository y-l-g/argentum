// Dialog dismissal for SSR dialogs — the server renders `<dialog open>`, so
// the closed state is normally a navigation (Cancel/Delete are links). This
// script adds Escape, backdrop, and `[data-dialog-close]` dismissal without a
// reload, mirroring the closed state into the URL so a reload stays closed.
//
// Document-level delegation (like bulk.js) so a dialog that arrives in
// streamed or shard-swapped markup dismisses too — binding at
// DOMContentLoaded missed anything the server rendered later.
function dismissDialog(dialog) {
  if (!dialog.open) return;
  dialog.close();
  const url = new URL(window.location.href);
  url.searchParams.set('open', 'false');
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
