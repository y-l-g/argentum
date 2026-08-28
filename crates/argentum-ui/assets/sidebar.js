document.addEventListener('DOMContentLoaded', () => {
  const s = document.querySelector('[data-sidebar="sidebar"]');
  const p = document.querySelector('[data-sidebar="provider"]');
  const sheet = document.getElementById('mobile-sidebar-sheet');
  if (s && p) {
    document.addEventListener('click', e => {
      if (e.target.closest('[data-sidebar="trigger"]')) {
        if (window.innerWidth < 1024 && sheet) {
          if (sheet.hasAttribute('open')) {
            sheet.removeAttribute('open');
            sheet.close?.();
          } else {
            sheet.setAttribute('open', '');
            sheet.showModal?.();
          }
        } else {
          const c = s.getAttribute('data-state') === 'collapsed' ? 'expanded' : 'collapsed';
          s.setAttribute('data-state', c);
          p.setAttribute('data-state', c);
          document.cookie = `sidebar_state=${c};path=/;max-age=604800`;
        }
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
  // Close sheet when clicking backdrop (dialog's ::backdrop)
  if (sheet) {
    sheet.addEventListener('click', e => {
      if (e.target === sheet) sheet.removeAttribute('open');
    });
  }
});
