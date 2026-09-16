// Toast lifecycle for Argentum shells (GH #97, GH #151).
//
// The shell renders shadcn/Sonner toast surfaces as `[data-sonner-toast]`
// inside a polite live region. Each toast ships `data-mounted="false"` (slid
// down and transparent) so this script can mount it and play Sonner's enter
// transition; auto-dismissal mirrors Sonner (4s, paused while hovered or
// focused) and exits through `data-removed="true"`. The close button carries
// Sonner's `[data-close-button]`. Without JS the `<noscript>` rule in the
// toaster keeps the toast visible until the next navigation.
//
// Document-level delegation (like bulk.js) so toasts swapped in by streamed
// `suspense` regions need no re-installation.
const TOAST_LIFETIME = 4000;
// Equal to the exit transition in the toast surface (400ms).
const TOAST_EXIT_MS = 400;

function dismissToast(el) {
  if (el.dataset.removed === 'true') return;
  el.dataset.removed = 'true';
  window.setTimeout(() => el.remove(), TOAST_EXIT_MS);
}

function armToast(el) {
  if (el.dataset.notificationArmed) return;
  el.dataset.notificationArmed = 'true';
  // Flush the initial hidden style before flipping `data-mounted`; without
  // the reflow both changes coalesce and the enter transition never plays.
  void el.offsetHeight;
  el.dataset.mounted = 'true';

  // Sonner pauses the lifetime on hover/focus and resumes with the remaining
  // time, so a toast the reader is looking at does not vanish under them.
  let remaining = TOAST_LIFETIME;
  let startedAt = 0;
  let timer = null;
  const start = () => {
    startedAt = Date.now();
    timer = window.setTimeout(() => dismissToast(el), remaining);
  };
  const pause = () => {
    if (!timer) return;
    window.clearTimeout(timer);
    timer = null;
    remaining = Math.max(0, remaining - (Date.now() - startedAt));
  };
  el.addEventListener('mouseenter', pause);
  el.addEventListener('mouseleave', start);
  el.addEventListener('focusin', pause);
  el.addEventListener('focusout', start);
  start();
}

document.addEventListener('click', (e) => {
  const close = e.target.closest('[data-close-button]');
  if (!close) return;
  const toast = close.closest('[data-sonner-toast]');
  if (toast) dismissToast(toast);
});

document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-sonner-toast]').forEach(armToast);
});

// Catch toasts swapped in after load (streamed suspense regions).
if (typeof MutationObserver !== 'undefined') {
  const observer = new MutationObserver((mutations) => {
    for (const m of mutations) {
      for (const node of m.addedNodes) {
        if (node.nodeType !== 1) continue;
        if (node.matches && node.matches('[data-sonner-toast]')) armToast(node);
        if (node.querySelectorAll) {
          node.querySelectorAll('[data-sonner-toast]').forEach(armToast);
        }
      }
    }
  });
  if (document.documentElement) {
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }
}
