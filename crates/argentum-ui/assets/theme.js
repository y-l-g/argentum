document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-theme-toggle]').forEach(b => b.addEventListener('click', () => {
    const freeze = document.createElement('style');
    freeze.appendChild(document.createTextNode('*,*::before,*::after,*::backdrop{transition:none!important}'));
    document.head.appendChild(freeze);
    document.documentElement.classList.toggle('dark');
    const t = document.documentElement.classList.contains('dark') ? 'dark' : 'light';
    localStorage.setItem('theme', t);
    document.cookie = `theme=${t};path=/;max-age=31536000`;
    requestAnimationFrame(() => requestAnimationFrame(() => freeze.remove()));
  }));
  // Reconcile, don't just add (GH #184): the inline head script normally lands
  // this before first paint, so this is the backstop for a document that
  // reached the client some other way. A stored `light` must remove a
  // server-rendered `dark` class, or the choice is lost on the next page.
  const stored = localStorage.getItem('theme') || (document.cookie.match(/theme=([^;]+)/)?.[1]);
  if (stored === 'dark') document.documentElement.classList.add('dark');
  else if (stored === 'light') document.documentElement.classList.remove('dark');
});
