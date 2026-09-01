document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-copy-button]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const wrapper = btn.closest('.relative');
      // Prefer the visible <pre><code> — textContent of the first code block is the source.
      const codeEl = wrapper ? wrapper.querySelector('pre code') : null;
      const text = codeEl ? codeEl.textContent : '';
      try {
        await navigator.clipboard.writeText(text);
        const prev = btn.textContent;
        btn.textContent = 'Copied!';
        setTimeout(() => (btn.textContent = prev), 1500);
      } catch {}
    });
  });
});
