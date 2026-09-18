// Copy buttons for Argentum code blocks.
//
// `code_block` renders a `button[data-copy-button]` beside every `<pre><code>`;
// clicking it copies the code element's text through the clipboard API with a
// transient "Copied!" label. The gutter numbers live outside the `<code>` so
// they stay chrome, never copied text.
//
// Document-level delegation (like bulk.js) so code blocks swapped in by
// streamed `suspense` regions or shard re-renders copy too — binding each
// button at DOMContentLoaded missed anything the server rendered later
// (GH #152). The handler is `async` but never awaits before resolving the
// button, so the clipboard write keeps its user gesture.
document.addEventListener('click', async (e) => {
  const btn = e.target.closest('[data-copy-button]');
  if (!btn) return;
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
