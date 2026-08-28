document.addEventListener('DOMContentLoaded', () => {
  const s = document.querySelector('[data-sidebar="sidebar"]');
  const p = document.querySelector('[data-sidebar="provider"]');
  const sheet = document.getElementById('mobile-sidebar-sheet');
  if (s && p) {
    const setState = state => {
      const collapsible = state === 'collapsed' ? 'offcanvas' : '';
      s.setAttribute('data-state', state);
      s.setAttribute('data-collapsible', collapsible);
      p.setAttribute('data-state', state);
      p.setAttribute('data-collapsible', collapsible);
      document.cookie = `sidebar_state=${state};path=/;max-age=604800`;
    };
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
          setState(s.getAttribute('data-state') === 'collapsed' ? 'expanded' : 'collapsed');
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
    if (m) setState(m[1]);
  }
  if (sheet) {
    // Close the sheet when clicking its backdrop (the <dialog> element itself)
    sheet.addEventListener('click', e => {
      if (e.target === sheet) {
        sheet.removeAttribute('open');
        sheet.close?.();
      }
    });
  }
});
