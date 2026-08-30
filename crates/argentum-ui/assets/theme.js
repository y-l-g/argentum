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
  const t = localStorage.getItem('theme') || (document.cookie.match(/theme=([^;]+)/)?.[1]);
  if (t === 'dark') document.documentElement.classList.add('dark');
});
