// Toast lifecycle for Argentum shells.
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
  // Hover and focus are independent pause sources: the countdown
  // runs only while *none* of them is active, and a resume from one leaves
  // the countdown stopped while another still holds it. One timer, always
  // cleared before the next is armed.
  let remaining = TOAST_LIFETIME;
  let startedAt = 0;
  let timer = null;
  const pauses = new Set();
  const start = () => {
    if (pauses.size > 0 || timer !== null) return;
    startedAt = Date.now();
    timer = window.setTimeout(() => dismissToast(el), remaining);
  };
  const pause = (source) => {
    if (pauses.has(source)) return;
    pauses.add(source);
    if (timer === null) return;
    window.clearTimeout(timer);
    timer = null;
    remaining = Math.max(0, remaining - (Date.now() - startedAt));
  };
  const resume = (source) => {
    if (!pauses.delete(source)) return;
    start();
  };
  el.addEventListener('mouseenter', () => pause('hover'));
  el.addEventListener('mouseleave', () => resume('hover'));
  el.addEventListener('focusin', () => pause('focus'));
  el.addEventListener('focusout', (e) => {
    // Focus moving inside the toast is still focus on it; only leaving resumes.
    if (!el.contains || !el.contains(e.relatedTarget)) resume('focus');
  });
  start();
}

// Everything below only makes sense with a document. It lives in a function so
// this file can also be `require`d by its Node unit test
// (`notifications.test.js`), which has no DOM: loading the script must not
// touch one.
function install() {
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
  if (typeof MutationObserver !== 'undefined' && document.documentElement) {
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
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`notifications.test.js`); see `bulk.js` for
// the guard.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { TOAST_EXIT_MS, TOAST_LIFETIME, armToast, dismissToast };
}
