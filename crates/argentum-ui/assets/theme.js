document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-theme-toggle]').forEach(b => b.addEventListener('click', () => {
    document.documentElement.classList.toggle('dark');
    const t = document.documentElement.classList.contains('dark') ? 'dark' : 'light';
    localStorage.setItem('theme', t);
    document.cookie = `theme=${t};path=/;max-age=31536000`;
  }));
  const t = localStorage.getItem('theme') || (document.cookie.match(/theme=([^;]+)/)?.[1]);
  if (t === 'dark') document.documentElement.classList.add('dark');
});
