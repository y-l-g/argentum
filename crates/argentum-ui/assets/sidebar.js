document.addEventListener('DOMContentLoaded', () => {
  const s = document.querySelector('[data-sidebar="sidebar"]');
  const p = document.querySelector('[data-sidebar="provider"]');
  if (s && p) {
    document.addEventListener('click', e => {
      if (e.target.closest('[data-sidebar="trigger"]')) {
        const c = s.getAttribute('data-state') === 'collapsed' ? 'expanded' : 'collapsed';
        s.setAttribute('data-state', c);
        p.setAttribute('data-state', c);
        document.cookie = `sidebar_state=${c};path=/;max-age=604800`;
      }
    });
    document.addEventListener('keydown', e => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
        e.preventDefault();
        document.querySelector('[data-sidebar="trigger"]')?.click();
      }
    });
    const m = document.cookie.match(/sidebar_state=([^;]+)/);
    if (m && s && p) {
      s.setAttribute('data-state', m[1]);
      p.setAttribute('data-state', m[1]);
    }
  }
});
