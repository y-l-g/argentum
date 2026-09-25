// sidebar.js — persistence and keyboard shortcut for the shell sidebar.
//
// The sidebar's open state is Topcoat runtime state (signals created in
// `Panel::render_shell`): the triggers carry `@click` handlers and the
// panel's `data-state` changes in the browser. This script only mirrors the
// persisted desktop state into the `sidebar_state` cookie, which seeds the
// signal on the next server render, and maps Ctrl/Cmd+B to the visible
// trigger.
//
// Hooks: `[data-sidebar="sidebar"]` (its `data-state`),
// `[data-sidebar="trigger"]` (click).
document.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
    e.preventDefault();
    const trigger = [...document.querySelectorAll('[data-sidebar="trigger"]')].find(
      el => el.getClientRects().length > 0,
    );
    trigger?.click();
  }
});

// Observe the whole document: the runtime morphs swapped content and page
// re-runs replace the panel, so a listener bound to the current element
// would not survive. Only the desktop panel is persisted; the mobile sheet
// is transient.
new MutationObserver(records => {
  for (const { target } of records) {
    if (target.getAttribute?.('data-sidebar') !== 'sidebar') continue;
    const state = target.getAttribute('data-state');
    if (state === 'expanded' || state === 'collapsed') {
      document.cookie = `sidebar_state=${state};path=/;max-age=604800`;
    }
  }
}).observe(document.documentElement, {
  attributes: true,
  attributeFilter: ['data-state'],
  subtree: true,
});
