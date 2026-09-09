// Notification auto-dismiss for Argentum shells (GH #97).
//
// The shell renders each toast as `[data-notification]` inside a
// `role=status` live region. Toasts persist until next navigation without JS;
// this listener fades them out after ~4s and supports manual dismiss via
// `[data-notification-close]`. Document-level delegation (like bulk.js) so
// streamed `suspense` swaps that replace shell markup need no re-installation.
function armNotification(el) {
  if (el.dataset.notificationArmed) return;
  el.dataset.notificationArmed = 'true';
  window.setTimeout(() => {
    el.style.transition = 'opacity 0.3s';
    el.style.opacity = '0';
    window.setTimeout(() => el.remove(), 320);
  }, 4000);
}

document.addEventListener('click', (e) => {
  const close = e.target.closest('[data-notification-close]');
  if (close) {
    const toast = close.closest('[data-notification]');
    if (toast) toast.remove();
  }
});

document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-notification]').forEach(armNotification);
});

// Catch toasts swapped in after load (streamed suspense regions).
if (typeof MutationObserver !== 'undefined') {
  const observer = new MutationObserver((mutations) => {
    for (const m of mutations) {
      for (const node of m.addedNodes) {
        if (node.nodeType !== 1) continue;
        if (node.matches && node.matches('[data-notification]')) armNotification(node);
        if (node.querySelectorAll) {
          node.querySelectorAll('[data-notification]').forEach(armNotification);
        }
      }
    }
  });
  if (document.documentElement) {
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }
}
