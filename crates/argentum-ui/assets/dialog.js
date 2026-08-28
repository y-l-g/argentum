// Dialog dismissal for SSR dialogs — the server renders `<dialog open>`, so
// the closed state is normally a navigation (Cancel/Delete are links). This
// script adds Escape, backdrop, and `[data-dialog-close]` dismissal without a
// reload, mirroring the closed state into the URL so a reload stays closed.
document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('dialog[open]').forEach(d => {
    const dismiss = () => {
      if (!d.open) return;
      d.close();
      const url = new URL(window.location.href);
      url.searchParams.set('open', 'false');
      window.history.pushState(window.history.state, '', url);
    };
    d.addEventListener('click', e => {
      // The overlay is the <dialog> itself; a click on it (not the panel
      // inside) is the backdrop.
      if (e.target === d) {
        dismiss();
        return;
      }
      if (e.target.closest('[data-dialog-close]')) {
        e.preventDefault();
        dismiss();
      }
    });
    // Modal dialogs fire `cancel` on Escape. Non-modal ones (SSR
    // `<dialog open>`) do not, so the keydown bridge below re-dispatches it.
    d.addEventListener('cancel', e => {
      e.preventDefault();
      dismiss();
    });
  });
  document.addEventListener('keydown', e => {
    if (e.key !== 'Escape') return;
    document.querySelector('dialog[open]')?.dispatchEvent(new Event('cancel'));
  });
});
